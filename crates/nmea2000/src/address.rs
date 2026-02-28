//! ISO 11783-5 Address Claiming for NMEA 2000.
//!
//! The [`AddressManager`] is a standalone state machine that manages device
//! addresses on the bus. It processes incoming messages and produces
//! [`AddressAction`]s that the caller sends via their transport.
//!
//! # Usage
//!
//! ```no_run
//! use nmea2000::address::{AddressManager, AddressAction};
//! use nmea2000_types::{IsoNameBuilder, RawMessage};
//! use std::time::Instant;
//!
//! let name = IsoNameBuilder::new()
//!     .unique_number(12345)
//!     .manufacturer_code(2047)
//!     .device_function(130)
//!     .device_class(25)
//!     .industry_group(4)
//!     .arbitrary_address_capable(true)
//!     .build();
//!
//! let mut mgr = AddressManager::new();
//! mgr.add_device(name, 100); // preferred address 100
//!
//! // In your main loop:
//! // 1. Feed incoming messages: mgr.process(&msg);
//! // 2. Check timeouts: mgr.check_timeouts(Instant::now());
//! // 3. Drain actions and send them on the bus.
//! ```

use std::collections::VecDeque;
use std::time::Instant;

use nmea2000_types::{IsoName, Pgn, RawMessage};

/// PGN 60928 — ISO Address Claim.
const PGN_ADDRESS_CLAIM: u32 = 60928;
/// PGN 59904 — ISO Request.
const PGN_ISO_REQUEST: u32 = 59904;
/// Cannot Claim address (source 254).
const CANNOT_CLAIM_ADDRESS: u8 = 254;
/// Claim timeout: 250 ms per ISO 11783-5.
const CLAIM_TIMEOUT_MS: u128 = 250;

/// Actions produced by the [`AddressManager`].
///
/// The caller is responsible for converting these into actual bus messages
/// using their transport.
#[derive(Debug, Clone)]
pub enum AddressAction {
    /// Send an Address Claim (PGN 60928) from the given source address.
    SendClaim {
        /// Source address to claim.
        source: u8,
        /// The NAME being claimed.
        name: IsoName,
    },
    /// Send a Cannot Claim (PGN 60928 from address 254).
    SendCannotClaim {
        /// The NAME that cannot claim an address.
        name: IsoName,
    },
    /// A device has successfully claimed an address (250ms elapsed).
    AddressClaimed {
        /// The claimed address.
        address: u8,
        /// The NAME that claimed it.
        name: IsoName,
    },
    /// A device lost its address due to arbitration.
    AddressLost {
        /// The address that was lost.
        old_address: u8,
        /// The NAME that lost the address.
        name: IsoName,
    },
}

/// Build a [`RawMessage`] for PGN 60928 (Address Claim).
#[must_use]
pub fn build_address_claim(source: u8, name: IsoName) -> RawMessage {
    RawMessage {
        pgn: Pgn::new(PGN_ADDRESS_CLAIM),
        source,
        destination: None, // broadcast
        priority: 6,
        data: name.to_bytes().to_vec(),
    }
}

/// Build a [`RawMessage`] for Cannot Claim (PGN 60928 from address 254).
#[must_use]
pub fn build_cannot_claim(name: IsoName) -> RawMessage {
    RawMessage {
        pgn: Pgn::new(PGN_ADDRESS_CLAIM),
        source: CANNOT_CLAIM_ADDRESS,
        destination: None, // broadcast
        priority: 6,
        data: name.to_bytes().to_vec(),
    }
}

#[derive(Debug, Clone)]
enum DeviceState {
    /// Claim sent, waiting for 250ms timeout.
    Claiming { address: u8, deadline: Instant },
    /// Address successfully claimed.
    Claimed { address: u8 },
    /// Lost address, trying a new one.
    Reclaiming { address: u8, deadline: Instant },
    /// No free address available.
    CannotClaim,
}

#[derive(Debug, Clone)]
struct DeviceEntry {
    name: IsoName,
    state: DeviceState,
}

/// Tracks which NAME holds each address (0–253).
#[derive(Debug)]
pub struct AddressTable {
    entries: [Option<IsoName>; 254],
}

impl AddressTable {
    fn new() -> Self {
        Self {
            entries: [None; 254],
        }
    }

    /// Get the NAME at a given address.
    #[must_use]
    pub fn name_at(&self, address: u8) -> Option<IsoName> {
        if address < 254 {
            self.entries[address as usize]
        } else {
            None
        }
    }

    fn set(&mut self, address: u8, name: IsoName) {
        if address < 254 {
            self.entries[address as usize] = Some(name);
        }
    }

    fn clear(&mut self, address: u8) {
        if address < 254 {
            self.entries[address as usize] = None;
        }
    }

    /// Find a free address, starting from `start` and wrapping around.
    fn find_free(&self, start: u8) -> Option<u8> {
        // Search from start..254, then 0..start
        for i in 0..254u16 {
            let addr = ((start as u16 + i) % 254) as u8;
            if self.entries[addr as usize].is_none() {
                return Some(addr);
            }
        }
        None
    }
}

/// ISO 11783-5 Address Claiming state machine.
///
/// Manages one or more devices on the NMEA 2000 bus. Feed incoming
/// messages via [`process`](Self::process), check timeouts via
/// [`check_timeouts`](Self::check_timeouts), and drain resulting
/// actions via [`drain_actions`](Self::drain_actions).
pub struct AddressManager {
    devices: Vec<DeviceEntry>,
    bus_table: AddressTable,
    pending: VecDeque<AddressAction>,
}

impl AddressManager {
    /// Create a new address manager with no devices.
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            bus_table: AddressTable::new(),
            pending: VecDeque::new(),
        }
    }

    /// Register a device with a preferred address.
    ///
    /// Immediately produces a `SendClaim` action. The claim is considered
    /// successful after 250ms if no competing claim is received.
    pub fn add_device(&mut self, name: IsoName, preferred_address: u8) {
        let now = Instant::now();
        let address =
            if preferred_address < 254 && self.bus_table.name_at(preferred_address).is_none() {
                preferred_address
            } else {
                // Preferred address taken, find a free one
                let Some(addr) = self.bus_table.find_free(preferred_address) else {
                    self.devices.push(DeviceEntry {
                        name,
                        state: DeviceState::CannotClaim,
                    });
                    self.pending
                        .push_back(AddressAction::SendCannotClaim { name });
                    return;
                };
                addr
            };

        self.bus_table.set(address, name);
        self.devices.push(DeviceEntry {
            name,
            state: DeviceState::Claiming {
                address,
                deadline: now + std::time::Duration::from_millis(CLAIM_TIMEOUT_MS as u64),
            },
        });
        self.pending.push_back(AddressAction::SendClaim {
            source: address,
            name,
        });
    }

    /// Process an incoming message.
    ///
    /// Handles PGN 60928 (Address Claim) and PGN 59904 (ISO Request).
    /// All messages are passed through — nothing is consumed.
    pub fn process(&mut self, msg: &RawMessage) {
        match msg.pgn.as_u32() {
            PGN_ADDRESS_CLAIM => self.handle_address_claim(msg),
            PGN_ISO_REQUEST => self.handle_iso_request(msg),
            _ => {}
        }
    }

    /// Check for expired claim timeouts.
    ///
    /// Call this periodically (e.g., every 50ms) to detect when
    /// the 250ms claim window has elapsed.
    pub fn check_timeouts(&mut self, now: Instant) {
        for device in &mut self.devices {
            match device.state {
                DeviceState::Claiming { address, deadline }
                | DeviceState::Reclaiming { address, deadline }
                    if now >= deadline =>
                {
                    device.state = DeviceState::Claimed { address };
                    self.pending.push_back(AddressAction::AddressClaimed {
                        address,
                        name: device.name,
                    });
                }
                _ => {}
            }
        }
    }

    /// Drain all pending actions.
    pub fn drain_actions(&mut self) -> impl Iterator<Item = AddressAction> + '_ {
        self.pending.drain(..)
    }

    /// Get the currently claimed address for a NAME, if any.
    #[must_use]
    pub fn address_for(&self, name: IsoName) -> Option<u8> {
        self.devices.iter().find_map(|d| {
            if d.name == name {
                match d.state {
                    DeviceState::Claimed { address }
                    | DeviceState::Claiming { address, .. }
                    | DeviceState::Reclaiming { address, .. } => Some(address),
                    DeviceState::CannotClaim => None,
                }
            } else {
                None
            }
        })
    }

    /// Access the bus address table.
    #[must_use]
    pub fn bus_table(&self) -> &AddressTable {
        &self.bus_table
    }

    fn handle_address_claim(&mut self, msg: &RawMessage) {
        if msg.data.len() < 8 {
            return;
        }

        let claimed_name =
            IsoName::from_bytes(msg.data[..8].try_into().expect("slice is exactly 8 bytes"));
        let claimed_address = msg.source;

        // Update bus table for external devices
        if claimed_address < 254 {
            self.bus_table.set(claimed_address, claimed_name);
        }

        // Check if any of our devices are affected
        for device in &mut self.devices {
            let our_address = match device.state {
                DeviceState::Claiming { address, .. }
                | DeviceState::Reclaiming { address, .. }
                | DeviceState::Claimed { address } => address,
                DeviceState::CannotClaim => continue,
            };

            if our_address != claimed_address {
                continue;
            }

            // Someone is claiming the same address as us
            if claimed_name == device.name {
                // It's our own claim echoed back — ignore
                continue;
            }

            if device.name < claimed_name {
                // We have higher priority (lower NAME) — re-assert our claim
                self.pending.push_back(AddressAction::SendClaim {
                    source: our_address,
                    name: device.name,
                });
            } else {
                // We lose — must find a new address
                let old_address = our_address;
                self.bus_table.clear(old_address);
                self.pending.push_back(AddressAction::AddressLost {
                    old_address,
                    name: device.name,
                });

                if device.name.arbitrary_address_capable() {
                    if let Some(new_addr) = self.bus_table.find_free(old_address.wrapping_add(1)) {
                        let deadline = Instant::now()
                            + std::time::Duration::from_millis(CLAIM_TIMEOUT_MS as u64);
                        self.bus_table.set(new_addr, device.name);
                        device.state = DeviceState::Reclaiming {
                            address: new_addr,
                            deadline,
                        };
                        self.pending.push_back(AddressAction::SendClaim {
                            source: new_addr,
                            name: device.name,
                        });
                    } else {
                        device.state = DeviceState::CannotClaim;
                        self.pending
                            .push_back(AddressAction::SendCannotClaim { name: device.name });
                    }
                } else {
                    device.state = DeviceState::CannotClaim;
                    self.pending
                        .push_back(AddressAction::SendCannotClaim { name: device.name });
                }
            }
        }
    }

    fn handle_iso_request(&mut self, msg: &RawMessage) {
        if msg.data.len() < 3 {
            return;
        }

        let requested_pgn =
            u32::from(msg.data[0]) | (u32::from(msg.data[1]) << 8) | (u32::from(msg.data[2]) << 16);

        if requested_pgn != PGN_ADDRESS_CLAIM {
            return;
        }

        // Respond with our current claims
        for device in &self.devices {
            match device.state {
                DeviceState::Claimed { address }
                | DeviceState::Claiming { address, .. }
                | DeviceState::Reclaiming { address, .. } => {
                    self.pending.push_back(AddressAction::SendClaim {
                        source: address,
                        name: device.name,
                    });
                }
                DeviceState::CannotClaim => {
                    self.pending
                        .push_back(AddressAction::SendCannotClaim { name: device.name });
                }
            }
        }
    }
}

impl Default for AddressManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nmea2000_types::IsoNameBuilder;

    fn make_name(unique: u32, mfg: u16, aac: bool) -> IsoName {
        IsoNameBuilder::new()
            .unique_number(unique)
            .manufacturer_code(mfg)
            .device_function(130)
            .device_class(25)
            .industry_group(4)
            .arbitrary_address_capable(aac)
            .build()
    }

    fn make_claim(source: u8, name: IsoName) -> RawMessage {
        build_address_claim(source, name)
    }

    fn drain(mgr: &mut AddressManager) -> Vec<AddressAction> {
        mgr.drain_actions().collect()
    }

    #[test]
    fn claim_no_conflict() {
        let name = make_name(100, 275, true);
        let mut mgr = AddressManager::new();
        mgr.add_device(name, 42);

        let actions = drain(&mut mgr);
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            AddressAction::SendClaim { source: 42, .. }
        ));

        // After timeout
        let deadline = Instant::now() + std::time::Duration::from_millis(300);
        mgr.check_timeouts(deadline);

        let actions = drain(&mut mgr);
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            AddressAction::AddressClaimed { address: 42, .. }
        ));

        assert_eq!(mgr.address_for(name), Some(42));
    }

    #[test]
    fn claim_conflict_we_win() {
        let our_name = make_name(100, 275, true); // Lower = higher priority
        let their_name = make_name(200, 275, true); // Higher = lower priority

        let mut mgr = AddressManager::new();
        mgr.add_device(our_name, 42);
        drain(&mut mgr); // Consume initial SendClaim

        // They claim the same address
        let their_claim = make_claim(42, their_name);
        mgr.process(&their_claim);

        let actions = drain(&mut mgr);
        // We should re-send our claim (we have lower NAME = higher priority)
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            AddressAction::SendClaim { source: 42, .. }
        ));
    }

    #[test]
    fn claim_conflict_we_lose() {
        let our_name = make_name(200, 275, true); // Higher = lower priority
        let their_name = make_name(100, 275, true); // Lower = higher priority

        let mut mgr = AddressManager::new();
        mgr.add_device(our_name, 42);
        drain(&mut mgr);

        // They claim the same address with higher priority
        let their_claim = make_claim(42, their_name);
        mgr.process(&their_claim);

        let actions = drain(&mut mgr);
        // We should lose and reclaim on a new address
        assert!(actions.len() >= 2);
        assert!(matches!(
            &actions[0],
            AddressAction::AddressLost {
                old_address: 42,
                ..
            }
        ));
        assert!(matches!(&actions[1], AddressAction::SendClaim { .. }));

        // Verify new address is different
        if let AddressAction::SendClaim { source, .. } = &actions[1] {
            assert_ne!(*source, 42);
        }
    }

    #[test]
    fn claim_conflict_not_aac_cannot_claim() {
        let our_name = make_name(200, 275, false); // NOT arbitrary address capable
        let their_name = make_name(100, 275, false); // Lower NAME = higher priority

        let mut mgr = AddressManager::new();
        mgr.add_device(our_name, 42);
        drain(&mut mgr);

        let their_claim = make_claim(42, their_name);
        mgr.process(&their_claim);

        let actions = drain(&mut mgr);
        assert!(actions.len() >= 2);
        assert!(matches!(&actions[0], AddressAction::AddressLost { .. }));
        assert!(matches!(&actions[1], AddressAction::SendCannotClaim { .. }));
    }

    #[test]
    fn iso_request_response() {
        let name = make_name(100, 275, true);
        let mut mgr = AddressManager::new();
        mgr.add_device(name, 42);

        let deadline = Instant::now() + std::time::Duration::from_millis(300);
        mgr.check_timeouts(deadline);
        drain(&mut mgr);

        // Someone requests PGN 60928
        let request = RawMessage {
            pgn: Pgn::new(PGN_ISO_REQUEST),
            source: 0x00,
            destination: None,
            priority: 6,
            data: vec![0x00, 0xEE, 0x00], // PGN 60928 LE
        };
        mgr.process(&request);

        let actions = drain(&mut mgr);
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            AddressAction::SendClaim { source: 42, .. }
        ));
    }

    #[test]
    fn multiple_devices() {
        let name1 = make_name(100, 275, true);
        let name2 = make_name(200, 275, true);

        let mut mgr = AddressManager::new();
        mgr.add_device(name1, 10);
        mgr.add_device(name2, 20);

        let actions = drain(&mut mgr);
        assert_eq!(actions.len(), 2);

        assert_eq!(mgr.address_for(name1), Some(10));
        assert_eq!(mgr.address_for(name2), Some(20));
    }

    #[test]
    fn build_claim_message() {
        let name = make_name(42, 275, true);
        let msg = build_address_claim(100, name);
        assert_eq!(msg.pgn.as_u32(), PGN_ADDRESS_CLAIM);
        assert_eq!(msg.source, 100);
        assert_eq!(msg.data, name.to_bytes());
    }

    #[test]
    fn build_cannot_claim_message() {
        let name = make_name(42, 275, true);
        let msg = build_cannot_claim(name);
        assert_eq!(msg.source, 254);
        assert_eq!(msg.data, name.to_bytes());
    }
}

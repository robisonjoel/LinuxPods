//! AAP battery status packet parsing.
//!
//! Format: 04 00 04 00 04 00 [count] ([component] 01 [level] [status] 01)...

/// Which physical component a battery reading belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Right,
    Left,
    Case,
    Unknown(u8),
}

impl From<u8> for Component {
    fn from(v: u8) -> Self {
        match v {
            2 => Self::Right,
            4 => Self::Left,
            8 => Self::Case,
            other => Self::Unknown(other),
        }
    }
}

impl std::fmt::Display for Component {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Right => write!(f, "Right"),
            Self::Left => write!(f, "Left"),
            Self::Case => write!(f, "Case"),
            Self::Unknown(_) => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Charging,
    Discharging,
    Disconnected,
    Unknown(u8),
}

impl From<u8> for Status {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Charging,
            2 => Self::Discharging,
            4 => Self::Disconnected,
            other => Self::Unknown(other),
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Charging => write!(f, "Charging"),
            Self::Discharging => write!(f, "Discharging"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Unknown(_) => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub component: Component,
    pub level: u8,
    pub status: Status,
}

impl Battery {
    pub fn is_charging(&self) -> bool {
        self.status == Status::Charging
    }

    /// The level, but only when it is a reading.
    ///
    /// A component the device reports as `Disconnected` still carries a level
    /// byte, and that byte is 0 - the case sends it as soon as the earbuds are
    /// out of it. Passing it through claims a flat battery. Levels above 100 are
    /// the other way the firmware says "no reading".
    pub fn available_level(&self) -> Option<u8> {
        if self.status == Status::Disconnected || self.level > 100 {
            return None;
        }
        Some(self.level)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatteryInfo {
    pub left: Option<Battery>,
    pub right: Option<Battery>,
    pub case: Option<Battery>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BatteryParseError {
    #[error("not a battery packet")]
    NotBatteryPacket,
    #[error("incomplete battery data at offset {0}")]
    Incomplete(usize),
}

/// Header check: 04 00 04 00 04 00 followed by a count byte.
pub fn is_battery_packet(packet: &[u8]) -> bool {
    packet.len() >= 7 && packet[..6] == [0x04, 0x00, 0x04, 0x00, 0x04, 0x00]
}

pub fn parse_battery_packet(packet: &[u8]) -> Result<BatteryInfo, BatteryParseError> {
    if !is_battery_packet(packet) {
        return Err(BatteryParseError::NotBatteryPacket);
    }

    let count = packet[6] as usize;
    let mut info = BatteryInfo::default();
    let mut offset = 7;

    for _ in 0..count {
        // Each entry is 5 bytes: [component] 01 [level] [status] 01
        if offset + 5 > packet.len() {
            return Err(BatteryParseError::Incomplete(offset));
        }

        let battery = Battery {
            component: Component::from(packet[offset]),
            level: packet[offset + 2],
            status: Status::from(packet[offset + 3]),
        };

        match battery.component {
            Component::Left => info.left = Some(battery),
            Component::Right => info.right = Some(battery),
            Component::Case => info.case = Some(battery),
            Component::Unknown(_) => {}
        }

        offset += 5;
    }

    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(entries: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut p = vec![0x04, 0x00, 0x04, 0x00, 0x04, 0x00, entries.len() as u8];
        for (component, level, status) in entries {
            p.extend_from_slice(&[*component, 0x01, *level, *status, 0x01]);
        }
        p
    }

    #[test]
    fn rejects_non_battery_packets() {
        assert!(!is_battery_packet(&[0x04, 0x00]));
        assert!(!is_battery_packet(&[
            0x04, 0x00, 0x04, 0x00, 0x31, 0x00, 0x01
        ]));
        assert_eq!(
            parse_battery_packet(&[0x00; 7]),
            Err(BatteryParseError::NotBatteryPacket)
        );
    }

    #[test]
    fn parses_all_three_components() {
        // 4 = left, 2 = right, 8 = case; status 1 = charging, 2 = discharging
        let p = packet(&[(4, 80, 2), (2, 75, 1), (8, 42, 2)]);
        let info = parse_battery_packet(&p).unwrap();

        let left = info.left.unwrap();
        assert_eq!(left.level, 80);
        assert_eq!(left.status, Status::Discharging);
        assert!(!left.is_charging());

        let right = info.right.unwrap();
        assert_eq!(right.level, 75);
        assert!(right.is_charging());

        assert_eq!(info.case.unwrap().level, 42);
    }

    #[test]
    fn tolerates_missing_components() {
        let info = parse_battery_packet(&packet(&[(4, 80, 2)])).unwrap();
        assert!(info.left.is_some());
        assert!(info.right.is_none());
        assert!(info.case.is_none());
    }

    #[test]
    fn detects_truncated_entries() {
        let mut p = packet(&[(4, 80, 2), (2, 75, 1)]);
        p.truncate(p.len() - 3); // cut into the second entry
        assert_eq!(
            parse_battery_packet(&p),
            Err(BatteryParseError::Incomplete(12))
        );
    }

    #[test]
    fn ignores_unknown_components() {
        let info = parse_battery_packet(&packet(&[(99, 50, 2)])).unwrap();
        assert_eq!(info, BatteryInfo::default());
    }

    #[test]
    fn a_disconnected_component_has_no_usable_level() {
        // Status 4 is what the case reports once the earbuds are out of it. The
        // level byte that rides along is 0, and 0 is not a reading - showing it
        // would claim a flat case.
        let info = parse_battery_packet(&packet(&[(8, 0, 4)])).unwrap();
        assert_eq!(info.case.unwrap().available_level(), None);
    }

    #[test]
    fn a_level_above_100_is_not_a_reading() {
        let info = parse_battery_packet(&packet(&[(8, 255, 2)])).unwrap();
        assert_eq!(info.case.unwrap().available_level(), None);
    }

    #[test]
    fn a_normal_reading_keeps_its_level() {
        let info = parse_battery_packet(&packet(&[(8, 42, 2)])).unwrap();
        assert_eq!(info.case.unwrap().available_level(), Some(42));
    }

    #[test]
    fn a_disconnected_component_is_not_charging() {
        let info = parse_battery_packet(&packet(&[(8, 0, 4)])).unwrap();
        assert!(!info.case.unwrap().is_charging());
    }
}

//! Shared Connector release identity and bounded, canonical numeric versions.

pub const CONNECTOR_VERSION: &str = "0.2.3";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConnectorVersion([u32; 3]);

impl ConnectorVersion {
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() > 32 {
            return None;
        }
        let mut parts = value.split('.');
        let mut numbers = [0; 3];
        for number in &mut numbers {
            let part = parts.next()?;
            if part.is_empty()
                || part.len() > 10
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            *number = part.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        Some(Self(numbers))
    }
}

pub fn version_status(reported: Option<&str>) -> &'static str {
    let Some(version) = reported.and_then(ConnectorVersion::parse) else {
        return "unknown";
    };
    let latest = ConnectorVersion::parse(CONNECTOR_VERSION).expect("canonical Connector version");
    match version.cmp(&latest) {
        std::cmp::Ordering::Less => "outdated",
        std::cmp::Ordering::Equal => "current",
        std::cmp::Ordering::Greater => "newer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_bounded_canonical_numeric_triples() {
        for value in [
            "0.0.0",
            CONNECTOR_VERSION,
            "4294967295.4294967295.4294967295",
        ] {
            assert!(ConnectorVersion::parse(value).is_some(), "{value}");
        }
        for value in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "-1.2.3",
            "+1.2.3",
            " 1.2.3",
            "1.2.3 ",
            "1.2.3-beta",
            "１.2.3",
            "4294967296.0.0",
            "00000000000.0.0",
        ] {
            assert!(ConnectorVersion::parse(value).is_none(), "{value}");
        }
        assert!(ConnectorVersion::parse("0.10.0") > ConnectorVersion::parse("0.9.99"));
        assert!(ConnectorVersion::parse("10.0.0") > ConnectorVersion::parse("2.99.99"));
        assert_eq!(version_status(None), "unknown");
        assert_eq!(version_status(Some("0.1.99")), "outdated");
        assert_eq!(version_status(Some(CONNECTOR_VERSION)), "current");
        assert_eq!(version_status(Some("0.10.0")), "newer");
        assert_eq!(version_status(Some("0.02.0")), "unknown");
    }
}

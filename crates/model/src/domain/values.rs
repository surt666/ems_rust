use serde::{Deserialize, Serialize};
use strum::{EnumIter, IntoEnumIterator};

// ---------------------------------------------------------------------------
// EdgeKind
// ---------------------------------------------------------------------------

/// The kind of a directed hierarchy edge.
#[derive(Debug, Clone, PartialEq, Eq, strum::Display)]
pub enum EdgeKind {
    #[strum(to_string = "has_label:{0}")]
    HasLabel(String),
    #[strum(serialize = "has_sensor")]
    HasSensor,
    #[strum(serialize = "blocked")]
    Blocked,
    #[strum(serialize = "administrates")]
    Administrates,
    #[strum(serialize = "reads")]
    Reads,
    #[strum(serialize = "writes")]
    Writes,
}

impl EdgeKind {
    /// The sort-key verb fragment used when writing the edge to DynamoDB.
    /// Only `HasLabel` deviates from `Display`; the rest equal their `Display`.
    pub fn sk_verb(&self) -> String {
        match self {
            EdgeKind::HasLabel(l) => format!("has_{l}"),
            other => other.to_string(),
        }
    }

    /// The serialised form stored in the `kind` attribute.
    pub fn kind_string(&self) -> String {
        self.to_string()
    }

    /// Reverse-direction verb used on `gsi1sk` for user-edge lookups.
    /// `HasLabel` and `HasSensor` return `None`; others carry a verb.
    pub const fn gsi_verb(&self) -> Option<&str> {
        match self {
            EdgeKind::Blocked => Some("blocks"),
            EdgeKind::Administrates => Some("administrators"),
            EdgeKind::Reads => Some("readers"),
            EdgeKind::Writes => Some("writers"),
            EdgeKind::HasLabel(_) | EdgeKind::HasSensor => None,
        }
    }

    /// Parse the serialised form produced by `Display`/`sk_verb`.
    pub fn parse(s: &str) -> Result<EdgeKind, String> {
        match s {
            "has_sensor" => Ok(EdgeKind::HasSensor),
            "blocked" => Ok(EdgeKind::Blocked),
            "administrates" => Ok(EdgeKind::Administrates),
            "reads" => Ok(EdgeKind::Reads),
            "writes" => Ok(EdgeKind::Writes),
            _ => match s.split_once(':') {
                Some(("has_label", l)) if !l.is_empty() => Ok(EdgeKind::HasLabel(l.to_string())),
                _ => Err(format!("bad edge_kind {:?}", s)),
            },
        }
    }

    /// Return the `CognitoGroup` capability this edge kind confers, if any.
    ///
    /// Only `Administrates`, `Reads`, and `Writes` confer a capability;
    /// `Blocked`, `HasLabel`, and `HasSensor` return `None`.
    pub const fn capability(&self) -> Option<CognitoGroup> {
        match self {
            EdgeKind::Administrates => Some(CognitoGroup::Admin),
            EdgeKind::Writes => Some(CognitoGroup::Writer),
            EdgeKind::Reads => Some(CognitoGroup::Reader),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CognitoGroup
// ---------------------------------------------------------------------------

/// The three real Cognito user-pool groups.
///
/// Canonical `to_string` is capitalised; `parse` is case-insensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString)]
#[strum(ascii_case_insensitive)]
pub enum CognitoGroup {
    #[strum(serialize = "Reader")]
    Reader,
    #[strum(serialize = "Writer")]
    Writer,
    #[strum(serialize = "Admin")]
    Admin,
}

impl CognitoGroup {
    /// Return the `EdgeKind` that should be used when granting access to a
    /// node for a user belonging to this group.
    ///
    /// `Admin → Administrates`, `Writer → Writes`, `Reader → Reads`.
    pub const fn access_edge(&self) -> EdgeKind {
        match self {
            CognitoGroup::Admin => EdgeKind::Administrates,
            CognitoGroup::Writer => EdgeKind::Writes,
            CognitoGroup::Reader => EdgeKind::Reads,
        }
    }

    /// Reverse of `EdgeKind::capability`: given an access edge kind return the
    /// corresponding group, or `None` for non-access edge kinds.
    pub const fn from_edge_kind(k: &EdgeKind) -> Option<CognitoGroup> {
        k.capability()
    }
}


// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

/// User-facing access profiles (UI/input concept, never stored directly).
///
/// Variant order is significant: Developer, Standard, Technician, Reader, Sysadm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter,
         strum::Display, strum::EnumString)]
pub enum Profile {
    #[strum(serialize = "Developer")]
    Developer,
    #[strum(serialize = "Standard")]
    Standard,
    #[strum(serialize = "Technician")]
    Technician,
    #[strum(serialize = "Reader")]
    Reader,
    #[strum(serialize = "SysAdm")]
    Sysadm,
}

impl Profile {
    /// All variants in order: Developer, Standard, Technician, Reader, Sysadm.
    pub fn all() -> Vec<Profile> {
        Profile::iter().collect()
    }

    /// Map a profile to its Cognito group.
    pub const fn to_cognito_group(&self) -> CognitoGroup {
        match self {
            Profile::Sysadm => CognitoGroup::Admin,
            Profile::Developer | Profile::Standard => CognitoGroup::Writer,
            Profile::Technician | Profile::Reader => CognitoGroup::Reader,
        }
    }
}


// ---------------------------------------------------------------------------
// Currency
// ---------------------------------------------------------------------------

/// Supported currencies.
///
/// Variant order backs the `render_currencies` option list: DKK, SEK, NOK, USD, EUR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default,
         strum::Display, strum::EnumString, EnumIter)]
pub enum Currency {
    /// Default currency.
    #[default]
    #[strum(serialize = "DKK")]
    Dkk,
    #[strum(serialize = "SEK")]
    Sek,
    #[strum(serialize = "NOK")]
    Nok,
    #[strum(serialize = "USD")]
    Usd,
    #[strum(serialize = "EUR")]
    Eur,
}


// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

/// Supported UI languages.
///
/// Variant order backs the `render_languages` option list: danish, swedish, norwegian, english, german.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default,
         strum::Display, strum::EnumString, EnumIter)]
pub enum Language {
    /// Default language.
    #[default]
    #[strum(serialize = "danish")]
    Danish,
    #[strum(serialize = "swedish")]
    Swedish,
    #[strum(serialize = "norwegian")]
    Norwegian,
    #[strum(serialize = "english")]
    English,
    #[strum(serialize = "german")]
    German,
}


// ---------------------------------------------------------------------------
// MeterType
// ---------------------------------------------------------------------------

/// Measurement accumulation style for a sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString)]
pub enum MeterType {
    #[strum(serialize = "counter")]
    Counter,
    #[strum(serialize = "gauge")]
    Gauge,
}


// ---------------------------------------------------------------------------
// Resource / Dimension
// ---------------------------------------------------------------------------

/// The accumulation dimension of a resource — the unit family it sums in.
///
/// Energy carriers roll up in kWh, volume carriers in m³. This is the read
/// side's `dimension_of_unit` grouping (the `measurements_aggregate` `gsi1`
/// dimension partition) made explicit in the domain. It is **derived** from the
/// resource (see [`Resource::dimension`]), never stored on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
pub enum Dimension {
    #[strum(serialize = "energy")]
    Energy,
    #[strum(serialize = "volume")]
    Volume,
}

/// The resource (energy form / EMS "Målertype") a sensor measures.
///
/// The string form (`strum` serialize, always lower-case) is the **wire/storage
/// contract**: it is written to the sensor's `purpose` attribute, forwarded to
/// `meter-identity`, and keyed verbatim into the `measurements_aggregate` rollup
/// sort key as the `<resource>` segment. The aggregations read side fans out over
/// these exact tokens, so this set must stay in lock-step with the rollup. Parsing
/// is case-insensitive (tolerates legacy rows); `Display` always re-emits the
/// canonical lower-case token, so writes are normalised.
///
/// Modelling `purpose` as this enum makes "a sensor measures a known resource" a
/// type-level invariant — illegal values can't be built, stored, or read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString, EnumIter, strum::IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum Resource {
    #[strum(serialize = "electricity")]
    Electricity,
    #[strum(serialize = "district_heating")]
    DistrictHeating,
    #[strum(serialize = "district_cooling")]
    DistrictCooling,
    #[strum(serialize = "gas")]
    Gas,
    #[strum(serialize = "water")]
    Water,
    #[strum(serialize = "heat")]
    Heat,
}

impl Resource {
    /// The canonical lower-case wire token (zero-alloc). Same string as
    /// `Display`; the read side keys rollup rows by this.
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Every resource, in declaration order — the per-node "all" fan-out set.
    /// Exposed as an inherent method so callers don't need `strum`'s iterator
    /// trait in scope.
    pub fn all() -> impl Iterator<Item = Resource> {
        Resource::iter()
    }

    /// The accumulation dimension — energy carriers (electricity, heat, district
    /// heating/cooling) sum in kWh; volume carriers (gas, water) in m³.
    pub const fn dimension(self) -> Dimension {
        match self {
            Resource::Electricity
            | Resource::DistrictHeating
            | Resource::DistrictCooling
            | Resource::Heat => Dimension::Energy,
            Resource::Gas | Resource::Water => Dimension::Volume,
        }
    }
}


// ---------------------------------------------------------------------------
// Timezone
// ---------------------------------------------------------------------------

/// Supported display timezones for the UI.
///
/// Variant order backs the `render_timezones` option list:
/// Copenhagen, Stockholm, Oslo, Berlin, London, Paris, Madrid, Rome, Amsterdam, Utc.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString, EnumIter)]
pub enum Timezone {
    #[strum(serialize = "Europe/Copenhagen")]
    Copenhagen,
    #[strum(serialize = "Europe/Stockholm")]
    Stockholm,
    #[strum(serialize = "Europe/Oslo")]
    Oslo,
    #[strum(serialize = "Europe/Berlin")]
    Berlin,
    #[strum(serialize = "Europe/London")]
    London,
    #[strum(serialize = "Europe/Paris")]
    Paris,
    #[strum(serialize = "Europe/Madrid")]
    Madrid,
    #[strum(serialize = "Europe/Rome")]
    Rome,
    #[strum(serialize = "Europe/Amsterdam")]
    Amsterdam,
    #[strum(serialize = "UTC")]
    Utc,
}


// ---------------------------------------------------------------------------
// Permission
// ---------------------------------------------------------------------------

/// UI permission levels used by the permissions dropdown.
///
/// Variant order backs the `render_permissions` option list: view, edit, admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString, EnumIter)]
pub enum Permission {
    #[strum(serialize = "view")]
    View,
    #[strum(serialize = "edit")]
    Edit,
    #[strum(serialize = "admin")]
    Admin,
}


// ---------------------------------------------------------------------------
// FieldType
// ---------------------------------------------------------------------------

/// Schema field type for metadata validation.
///
/// Integer bounds use `Option<i64>` and float bounds use `Option<f64>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FieldType {
    String {
        min_len: Option<i64>,
        max_len: Option<i64>,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Boolean,
    Timestamp,
    Enum {
        one_of: Vec<std::string::String>,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    // ---- Resource -----------------------------------------------------------

    /// `Display` re-emits the exact lower-case wire token for every variant —
    /// the `measurements_aggregate` rollup contract — and round-trips via parse.
    #[test]
    fn resource_wire_tokens_round_trip() {
        for r in Resource::iter() {
            assert_eq!(r.to_string().parse::<Resource>().unwrap(), r);
        }
        assert_eq!(Resource::DistrictHeating.to_string(), "district_heating");
        assert_eq!(Resource::Electricity.to_string(), "electricity");
    }

    /// Parsing tolerates legacy casing but `Display` always normalises.
    #[test]
    fn resource_parse_is_case_insensitive() {
        assert_eq!("Electricity".parse::<Resource>().unwrap(), Resource::Electricity);
        assert_eq!("WATER".parse::<Resource>().unwrap(), Resource::Water);
        assert!("energy".parse::<Resource>().is_err(), "dimension is not a resource");
    }

    /// Energy carriers sum in kWh, volume carriers in m³.
    #[test]
    fn resource_dimension_split() {
        assert_eq!(Resource::Electricity.dimension(), Dimension::Energy);
        assert_eq!(Resource::Heat.dimension(), Dimension::Energy);
        assert_eq!(Resource::Gas.dimension(), Dimension::Volume);
        assert_eq!(Resource::Water.dimension(), Dimension::Volume);
    }

    // ---- EdgeKind -----------------------------------------------------------

    #[test]
    fn edge_kind_has_label_verbs() {
        let k = EdgeKind::HasLabel("building".to_string());
        assert_eq!(k.sk_verb(), "has_building");
        assert_eq!(k.gsi_verb(), None);
    }

    #[test]
    fn edge_kind_has_sensor_verbs() {
        let k = EdgeKind::HasSensor;
        assert_eq!(k.sk_verb(), "has_sensor");
        assert_eq!(k.gsi_verb(), None);
    }

    #[test]
    fn edge_kind_blocked_verbs() {
        let k = EdgeKind::Blocked;
        assert_eq!(k.sk_verb(), "blocked");
        assert_eq!(k.gsi_verb(), Some("blocks"));
    }

    #[test]
    fn edge_kind_administrates_verbs() {
        let k = EdgeKind::Administrates;
        assert_eq!(k.sk_verb(), "administrates");
        assert_eq!(k.gsi_verb(), Some("administrators"));
    }

    #[test]
    fn edge_kind_reads_verbs() {
        let k = EdgeKind::Reads;
        assert_eq!(k.sk_verb(), "reads");
        assert_eq!(k.gsi_verb(), Some("readers"));
        assert_eq!(k.kind_string(), "reads");
    }

    #[test]
    fn edge_kind_writes_verbs() {
        let k = EdgeKind::Writes;
        assert_eq!(k.sk_verb(), "writes");
        assert_eq!(k.gsi_verb(), Some("writers"));
        assert_eq!(k.kind_string(), "writes");
    }

    #[test]
    fn edge_kind_roundtrip() {
        let kinds = vec![
            EdgeKind::HasLabel("b".to_string()),
            EdgeKind::HasSensor,
            EdgeKind::Blocked,
            EdgeKind::Administrates,
            EdgeKind::Reads,
            EdgeKind::Writes,
        ];
        for k in &kinds {
            let s = k.kind_string();
            let k2 = EdgeKind::parse(&s)
                .unwrap_or_else(|e| panic!("parse {:?}: {}", s, e));
            assert_eq!(k, &k2, "round-trip failed for {:?}", s);
        }
    }

    /// Plan example: sk_verb cases.
    #[test]
    fn edge_kind_sk_verb() {
        assert_eq!(EdgeKind::HasLabel("building".into()).sk_verb(), "has_building");
        assert_eq!(EdgeKind::HasSensor.sk_verb(), "has_sensor");
        assert_eq!(EdgeKind::Administrates.sk_verb(), "administrates");
        assert_eq!(EdgeKind::Blocked.sk_verb(), "blocked");
        assert_eq!(EdgeKind::Reads.sk_verb(), "reads");
        assert_eq!(EdgeKind::Writes.sk_verb(), "writes");
    }

    #[test]
    fn edge_kind_capability_roundtrip() {
        assert_eq!(EdgeKind::Administrates.capability(), Some(CognitoGroup::Admin));
        assert_eq!(EdgeKind::Writes.capability(), Some(CognitoGroup::Writer));
        assert_eq!(EdgeKind::Reads.capability(), Some(CognitoGroup::Reader));
        assert_eq!(EdgeKind::Blocked.capability(), None);
        assert_eq!(EdgeKind::HasSensor.capability(), None);
        assert_eq!(EdgeKind::HasLabel("x".into()).capability(), None);
    }

    /// Parse errors for bad inputs.
    #[test]
    fn edge_kind_parse_errors() {
        assert!(EdgeKind::parse("").is_err());
        assert!(EdgeKind::parse("has_label:").is_err()); // empty label
        assert!(EdgeKind::parse("unknown").is_err());
        assert!(EdgeKind::parse("has_label").is_err()); // no colon
    }

    /// Parse `"has_label:<l>"` with various labels.
    #[test]
    fn edge_kind_has_label_parse() {
        assert_eq!(
            EdgeKind::parse("has_label:floor").unwrap(),
            EdgeKind::HasLabel("floor".to_string())
        );
        assert_eq!(
            EdgeKind::parse("has_label:building").unwrap(),
            EdgeKind::HasLabel("building".to_string())
        );
    }

    // ---- CognitoGroup -------------------------------------------------------

    /// Plan example: capitalised Display + case-insensitive parse.
    #[test]
    fn cognito_group_capitalised_caseinsensitive() {
        assert_eq!(CognitoGroup::Admin.to_string(), "Admin");
        assert_eq!("admin".parse::<CognitoGroup>().unwrap(), CognitoGroup::Admin);
        assert_eq!("Admin".parse::<CognitoGroup>().unwrap(), CognitoGroup::Admin);
        assert_eq!("reader".parse::<CognitoGroup>().unwrap(), CognitoGroup::Reader);
    }

    #[test]
    fn cognito_group_parses() {
        assert_eq!("reader".parse::<CognitoGroup>().unwrap(), CognitoGroup::Reader);
        assert_eq!("writer".parse::<CognitoGroup>().unwrap(), CognitoGroup::Writer);
        assert_eq!("admin".parse::<CognitoGroup>().unwrap(), CognitoGroup::Admin);
        // canonical strings are capitalised
        assert_eq!(CognitoGroup::Admin.to_string(), "Admin");
        // of_string is case-insensitive
        assert_eq!("Admin".parse::<CognitoGroup>().unwrap(), CognitoGroup::Admin);
    }

    #[test]
    fn cognito_group_all_display() {
        assert_eq!(CognitoGroup::Reader.to_string(), "Reader");
        assert_eq!(CognitoGroup::Writer.to_string(), "Writer");
        assert_eq!(CognitoGroup::Admin.to_string(), "Admin");
    }

    #[test]
    fn cognito_group_parse_error() {
        assert!("superuser".parse::<CognitoGroup>().is_err());
        assert!("".parse::<CognitoGroup>().is_err());
    }

    #[test]
    fn cognito_group_access_edge_roundtrip() {
        // access_edge then capability forms a round-trip
        for (g, expected_kind) in [
            (CognitoGroup::Admin, EdgeKind::Administrates),
            (CognitoGroup::Writer, EdgeKind::Writes),
            (CognitoGroup::Reader, EdgeKind::Reads),
        ] {
            let kind = g.access_edge();
            assert_eq!(kind, expected_kind, "access_edge for {:?}", g);
            let back = kind.capability();
            assert_eq!(back, Some(g), "capability round-trip for {:?}", g);
            let back2 = CognitoGroup::from_edge_kind(&kind);
            assert_eq!(back2, Some(g), "from_edge_kind round-trip for {:?}", g);
        }
    }

    #[test]
    fn cognito_group_from_edge_kind_non_access() {
        assert_eq!(CognitoGroup::from_edge_kind(&EdgeKind::Blocked), None);
        assert_eq!(CognitoGroup::from_edge_kind(&EdgeKind::HasSensor), None);
    }

    // ---- Profile ------------------------------------------------------------

    /// Plan example: profile → cognito group mapping.
    #[test]
    fn profile_to_group() {
        use CognitoGroup::*;
        for (p, g) in [
            ("SysAdm", Admin),
            ("Developer", Writer),
            ("Standard", Writer),
            ("Technician", Reader),
            ("Reader", Reader),
        ] {
            assert_eq!(p.parse::<Profile>().unwrap().to_cognito_group(), g);
        }
    }

    #[test]
    fn profile_maps_to_groups() {
        let group_of = |s: &str| s.parse::<Profile>().unwrap().to_cognito_group();
        assert_eq!(group_of("SysAdm"), CognitoGroup::Admin);
        assert_eq!(group_of("Developer"), CognitoGroup::Writer);
        assert_eq!(group_of("Standard"), CognitoGroup::Writer);
        assert_eq!(group_of("Technician"), CognitoGroup::Reader);
        assert_eq!(group_of("Reader"), CognitoGroup::Reader);
    }

    #[test]
    fn profile_rejects_unknown() {
        assert!("Nope".parse::<Profile>().is_err());
    }

    #[test]
    fn profile_all_roundtrip() {
        for p in Profile::all() {
            let s = p.to_string();
            let p2 = s.parse::<Profile>()
                .unwrap_or_else(|e| panic!("roundtrip {:?}: {}", s, e));
            assert_eq!(p, p2);
        }
    }

    /// `all()` returns all 5 variants (order: Developer, Standard, Technician, Reader, Sysadm).
    #[test]
    fn profile_all_order() {
        let all = Profile::all();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0], Profile::Developer);
        assert_eq!(all[4], Profile::Sysadm);
    }

    // ---- Currency -----------------------------------------------------------

    #[test]
    fn currency_roundtrip() {
        let variants = [
            Currency::Dkk,
            Currency::Sek,
            Currency::Nok,
            Currency::Usd,
            Currency::Eur,
        ];
        for c in &variants {
            let s = c.to_string();
            assert_eq!(s.parse::<Currency>().unwrap(), *c);
        }
    }

    #[test]
    fn currency_strings() {
        assert_eq!(Currency::Dkk.to_string(), "DKK");
        assert_eq!(Currency::Sek.to_string(), "SEK");
        assert_eq!(Currency::Nok.to_string(), "NOK");
        assert_eq!(Currency::Usd.to_string(), "USD");
        assert_eq!(Currency::Eur.to_string(), "EUR");
    }

    #[test]
    fn currency_default_is_dkk() {
        assert_eq!(Currency::default(), Currency::Dkk);
    }

    #[test]
    fn currency_parse_error() {
        assert!("GBP".parse::<Currency>().is_err());
        assert!("dkk".parse::<Currency>().is_err()); // case-sensitive
    }

    // ---- Language -----------------------------------------------------------

    #[test]
    fn language_roundtrip() {
        let variants = [
            Language::Danish,
            Language::Swedish,
            Language::Norwegian,
            Language::English,
            Language::German,
        ];
        for l in &variants {
            let s = l.to_string();
            assert_eq!(s.parse::<Language>().unwrap(), *l);
        }
    }

    #[test]
    fn language_strings() {
        assert_eq!(Language::Danish.to_string(), "danish");
        assert_eq!(Language::Swedish.to_string(), "swedish");
        assert_eq!(Language::Norwegian.to_string(), "norwegian");
        assert_eq!(Language::English.to_string(), "english");
        assert_eq!(Language::German.to_string(), "german");
    }

    #[test]
    fn language_default_is_danish() {
        assert_eq!(Language::default(), Language::Danish);
    }

    #[test]
    fn language_parse_error() {
        assert!("French".parse::<Language>().is_err());
        assert!("Danish".parse::<Language>().is_err()); // case-sensitive
    }

    // ---- MeterType ----------------------------------------------------------

    #[test]
    fn meter_type_roundtrip() {
        for mt in [MeterType::Counter, MeterType::Gauge] {
            let s = mt.to_string();
            assert_eq!(s.parse::<MeterType>().unwrap(), mt);
        }
    }

    #[test]
    fn meter_type_strings() {
        assert_eq!(MeterType::Counter.to_string(), "counter");
        assert_eq!(MeterType::Gauge.to_string(), "gauge");
    }

    #[test]
    fn meter_type_parse_error() {
        assert!("Counter".parse::<MeterType>().is_err());
        assert!("unknown".parse::<MeterType>().is_err());
    }

    // ---- FieldType ----------------------------------------------------------

    #[test]
    fn field_type_serde_roundtrip() {
        let ft = FieldType::String {
            min_len: Some(2),
            max_len: None,
        };
        let json = serde_json::to_string(&ft).unwrap();
        let ft2: FieldType = serde_json::from_str(&json).unwrap();
        assert_eq!(ft, ft2);

        let ft_enum = FieldType::Enum {
            one_of: vec!["x".to_string()],
        };
        let json2 = serde_json::to_string(&ft_enum).unwrap();
        let ft_enum2: FieldType = serde_json::from_str(&json2).unwrap();
        assert_eq!(ft_enum, ft_enum2);
    }
}

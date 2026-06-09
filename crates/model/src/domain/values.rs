use std::fmt;
use serde::{Deserialize, Serialize};
use strum::EnumIter;

// ---------------------------------------------------------------------------
// EdgeKind
// ---------------------------------------------------------------------------

/// The kind of a directed hierarchy edge.
///
/// Faithfully ported from `services/hierarchy/lib/domain/edge_kind.ml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    HasLabel(String),
    HasSensor,
    Blocked,
    Administrates,
}

impl EdgeKind {
    /// The sort-key verb fragment used when writing the edge to DynamoDB.
    /// Matches OCaml `Edge_kind.sk_verb`.
    pub fn sk_verb(&self) -> String {
        match self {
            EdgeKind::HasLabel(l) => format!("has_{}", l),
            EdgeKind::HasSensor => "has_sensor".to_string(),
            EdgeKind::Blocked => "blocked".to_string(),
            EdgeKind::Administrates => "administrates".to_string(),
        }
    }

    /// The serialised form stored in the `kind` attribute.
    /// Matches OCaml `Edge_kind.to_string`.
    pub fn kind_string(&self) -> String {
        match self {
            EdgeKind::HasLabel(l) => format!("has_label:{}", l),
            EdgeKind::HasSensor => "has_sensor".to_string(),
            EdgeKind::Blocked => "blocked".to_string(),
            EdgeKind::Administrates => "administrates".to_string(),
        }
    }

    /// Reverse-direction verb used on `gsi1sk` for user-edge lookups.
    /// `HasLabel` and `HasSensor` return `None`; others carry a verb.
    /// Matches OCaml `Edge_kind.gsi_verb`.
    pub fn gsi_verb(&self) -> Option<&str> {
        match self {
            EdgeKind::Blocked => Some("blocks"),
            EdgeKind::Administrates => Some("administrators"),
            EdgeKind::HasLabel(_) | EdgeKind::HasSensor => None,
        }
    }

    /// Parse the serialised form produced by `kind_string` / OCaml `to_string`.
    /// Matches OCaml `Edge_kind.of_string`.
    pub fn parse(s: &str) -> Result<EdgeKind, String> {
        if s == "has_sensor" {
            return Ok(EdgeKind::HasSensor);
        }
        if s == "blocked" {
            return Ok(EdgeKind::Blocked);
        }
        if s == "administrates" {
            return Ok(EdgeKind::Administrates);
        }
        // Try "has_label:<l>"
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() == 2 && parts[0] == "has_label" && !parts[1].is_empty() {
            return Ok(EdgeKind::HasLabel(parts[1].to_string()));
        }
        Err(format!("bad edge_kind {:?}", s))
    }
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind_string())
    }
}

// ---------------------------------------------------------------------------
// CognitoGroup
// ---------------------------------------------------------------------------

/// The three real Cognito user-pool groups.
///
/// Faithfully ported from `services/hierarchy/lib/domain/cognito_group.ml`.
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
    /// Case-insensitive parse; accepts `"reader"`, `"Reader"`, `"READER"`, etc.
    /// Matches OCaml `Cognito_group.of_string`.
    pub fn parse(s: &str) -> Result<CognitoGroup, String> {
        s.parse::<CognitoGroup>().map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

/// User-facing access profiles (UI/input concept, never stored directly).
///
/// Faithfully ported from `services/hierarchy/lib/domain/profile.ml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter,
         strum::Display, strum::EnumString)]
pub enum Profile {
    #[strum(serialize = "SysAdm")]
    Sysadm,
    #[strum(serialize = "Developer")]
    Developer,
    #[strum(serialize = "Standard")]
    Standard,
    #[strum(serialize = "Technician")]
    Technician,
    #[strum(serialize = "Reader")]
    Reader,
}

impl Profile {
    /// All variants in OCaml list order: `[ Developer; Standard; Technician; Reader; Sysadm ]`.
    pub fn all() -> Vec<Profile> {
        // OCaml order: Developer, Standard, Technician, Reader, Sysadm
        // We use EnumIter but re-sort to match OCaml's `all` list order.
        // OCaml: let all = [ Developer; Standard; Technician; Reader; Sysadm ]
        vec![
            Profile::Developer,
            Profile::Standard,
            Profile::Technician,
            Profile::Reader,
            Profile::Sysadm,
        ]
    }

    /// Matches OCaml `Profile.to_string`.
    pub fn to_str(&self) -> &'static str {
        match self {
            Profile::Sysadm => "SysAdm",
            Profile::Developer => "Developer",
            Profile::Standard => "Standard",
            Profile::Technician => "Technician",
            Profile::Reader => "Reader",
        }
    }

    /// Case-sensitive parse, matching OCaml `Profile.of_string`.
    pub fn parse(s: &str) -> Result<Profile, String> {
        s.parse::<Profile>().map_err(|e| e.to_string())
    }

    /// Map a profile to its Cognito group.
    /// Matches OCaml `Profile.to_cognito_group`.
    pub fn to_cognito_group(&self) -> CognitoGroup {
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
/// Faithfully ported from `services/hierarchy/lib/domain/currency.ml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString)]
pub enum Currency {
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

impl Currency {
    /// Matches OCaml `Currency.to_string` (`"DKK"`, `"SEK"`, …).
    pub fn to_str(&self) -> &'static str {
        match self {
            Currency::Dkk => "DKK",
            Currency::Sek => "SEK",
            Currency::Nok => "NOK",
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
        }
    }

    /// Matches OCaml `Currency.of_string`.
    pub fn parse(s: &str) -> Result<Currency, String> {
        s.parse::<Currency>().map_err(|e| e.to_string())
    }
}

impl Default for Currency {
    /// Matches OCaml `Currency.default = DKK`.
    fn default() -> Self {
        Currency::Dkk
    }
}


// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

/// Supported UI languages.
///
/// Faithfully ported from `services/hierarchy/lib/domain/language.ml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString)]
pub enum Language {
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

impl Language {
    /// Matches OCaml `Language.to_string` (lowercase: `"danish"`, `"swedish"`, …).
    pub fn to_str(&self) -> &'static str {
        match self {
            Language::Danish => "danish",
            Language::Swedish => "swedish",
            Language::Norwegian => "norwegian",
            Language::English => "english",
            Language::German => "german",
        }
    }

    /// Matches OCaml `Language.of_string`.
    pub fn parse(s: &str) -> Result<Language, String> {
        s.parse::<Language>().map_err(|e| e.to_string())
    }
}

impl Default for Language {
    /// Matches OCaml `Language.default = Danish`.
    fn default() -> Self {
        Language::Danish
    }
}


// ---------------------------------------------------------------------------
// MeterType
// ---------------------------------------------------------------------------

/// Measurement accumulation style for a sensor.
///
/// Faithfully ported from `services/hierarchy/lib/domain/sensor.ml`
/// (`meter_type` type + `meter_type_to_string` / `meter_type_of_string`).
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString)]
pub enum MeterType {
    #[strum(serialize = "counter")]
    Counter,
    #[strum(serialize = "gauge")]
    Gauge,
}

impl MeterType {
    /// Matches OCaml `meter_type_to_string` (`"counter"` / `"gauge"`).
    pub fn to_str(&self) -> &'static str {
        match self {
            MeterType::Counter => "counter",
            MeterType::Gauge => "gauge",
        }
    }

    /// Matches OCaml `meter_type_of_string`.
    pub fn parse(s: &str) -> Result<MeterType, String> {
        s.parse::<MeterType>().map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// FieldType
// ---------------------------------------------------------------------------

/// Schema field type for metadata validation.
///
/// Faithfully ported from `services/hierarchy/lib/domain/metadata.ml`
/// (`field_type` variant definition only; validation functions are a later task).
///
/// OCaml `int option` maps to `Option<i64>` (the OCaml `Int64` / `int64` variant)
/// and `float option` to `Option<f64>`.
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
// Tests (ported 1:1 from OCaml test files + plan examples)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- EdgeKind -----------------------------------------------------------

    /// Port of `test_domain_edge_kind.ml :: has_label_verbs`
    #[test]
    fn edge_kind_has_label_verbs() {
        let k = EdgeKind::HasLabel("building".to_string());
        assert_eq!(k.sk_verb(), "has_building");
        assert_eq!(k.gsi_verb(), None);
    }

    /// Port of `test_domain_edge_kind.ml :: has_sensor_verbs`
    #[test]
    fn edge_kind_has_sensor_verbs() {
        let k = EdgeKind::HasSensor;
        assert_eq!(k.sk_verb(), "has_sensor");
        assert_eq!(k.gsi_verb(), None);
    }

    /// Port of `test_domain_edge_kind.ml :: blocked_verbs`
    #[test]
    fn edge_kind_blocked_verbs() {
        let k = EdgeKind::Blocked;
        assert_eq!(k.sk_verb(), "blocked");
        assert_eq!(k.gsi_verb(), Some("blocks"));
    }

    /// Port of `test_domain_edge_kind.ml :: administrates_verbs`
    #[test]
    fn edge_kind_administrates_verbs() {
        let k = EdgeKind::Administrates;
        assert_eq!(k.sk_verb(), "administrates");
        assert_eq!(k.gsi_verb(), Some("administrators"));
    }

    /// Port of `test_domain_edge_kind.ml :: roundtrip_to_string`
    #[test]
    fn edge_kind_roundtrip() {
        let kinds = vec![
            EdgeKind::HasLabel("b".to_string()),
            EdgeKind::HasSensor,
            EdgeKind::Blocked,
            EdgeKind::Administrates,
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
        assert_eq!(CognitoGroup::parse("admin").unwrap(), CognitoGroup::Admin);
        assert_eq!(CognitoGroup::parse("Admin").unwrap(), CognitoGroup::Admin);
        assert_eq!(CognitoGroup::parse("reader").unwrap(), CognitoGroup::Reader);
    }

    /// Port of `test_domain_user.ml :: cognito_group_parses`
    #[test]
    fn cognito_group_parses() {
        assert_eq!(CognitoGroup::parse("reader").unwrap(), CognitoGroup::Reader);
        assert_eq!(CognitoGroup::parse("writer").unwrap(), CognitoGroup::Writer);
        assert_eq!(CognitoGroup::parse("admin").unwrap(), CognitoGroup::Admin);
        // canonical strings are capitalised
        assert_eq!(CognitoGroup::Admin.to_string(), "Admin");
        // of_string is case-insensitive
        assert_eq!(CognitoGroup::parse("Admin").unwrap(), CognitoGroup::Admin);
    }

    #[test]
    fn cognito_group_all_display() {
        assert_eq!(CognitoGroup::Reader.to_string(), "Reader");
        assert_eq!(CognitoGroup::Writer.to_string(), "Writer");
        assert_eq!(CognitoGroup::Admin.to_string(), "Admin");
    }

    #[test]
    fn cognito_group_parse_error() {
        assert!(CognitoGroup::parse("superuser").is_err());
        assert!(CognitoGroup::parse("").is_err());
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
            assert_eq!(Profile::parse(p).unwrap().to_cognito_group(), g);
        }
    }

    /// Port of `test_domain_profile.ml :: maps_to_groups`
    #[test]
    fn profile_maps_to_groups() {
        let group_of = |s: &str| Profile::parse(s).unwrap().to_cognito_group();
        assert_eq!(group_of("SysAdm"), CognitoGroup::Admin);
        assert_eq!(group_of("Developer"), CognitoGroup::Writer);
        assert_eq!(group_of("Standard"), CognitoGroup::Writer);
        assert_eq!(group_of("Technician"), CognitoGroup::Reader);
        assert_eq!(group_of("Reader"), CognitoGroup::Reader);
    }

    /// Port of `test_domain_profile.ml :: rejects_unknown`
    #[test]
    fn profile_rejects_unknown() {
        assert!(Profile::parse("Nope").is_err());
    }

    /// Port of `test_domain_profile.ml :: all_roundtrip`
    #[test]
    fn profile_all_roundtrip() {
        for p in Profile::all() {
            let s = p.to_string();
            let p2 = Profile::parse(&s)
                .unwrap_or_else(|e| panic!("roundtrip {:?}: {}", s, e));
            assert_eq!(p, p2);
        }
    }

    /// `all()` returns all 5 variants (OCaml list order: Developer, Standard, Technician, Reader, Sysadm).
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
            assert_eq!(Currency::parse(&s).unwrap(), *c);
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
        assert!(Currency::parse("GBP").is_err());
        assert!(Currency::parse("dkk").is_err()); // case-sensitive
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
            assert_eq!(Language::parse(&s).unwrap(), *l);
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
        assert!(Language::parse("French").is_err());
        assert!(Language::parse("Danish").is_err()); // case-sensitive
    }

    // ---- MeterType ----------------------------------------------------------

    #[test]
    fn meter_type_roundtrip() {
        for mt in [MeterType::Counter, MeterType::Gauge] {
            let s = mt.to_string();
            assert_eq!(MeterType::parse(&s).unwrap(), mt);
        }
    }

    #[test]
    fn meter_type_strings() {
        assert_eq!(MeterType::Counter.to_string(), "counter");
        assert_eq!(MeterType::Gauge.to_string(), "gauge");
    }

    #[test]
    fn meter_type_parse_error() {
        assert!(MeterType::parse("Counter").is_err());
        assert!(MeterType::parse("unknown").is_err());
    }

    // ---- FieldType ----------------------------------------------------------

    #[test]
    fn field_type_string_variant() {
        let ft = FieldType::String {
            min_len: Some(1),
            max_len: Some(100),
        };
        // Just test it constructs and clones
        let ft2 = ft.clone();
        assert_eq!(ft, ft2);
    }

    #[test]
    fn field_type_number_variant() {
        let ft = FieldType::Number {
            min: Some(0.0),
            max: Some(100.0),
        };
        assert_eq!(ft, ft.clone());
    }

    #[test]
    fn field_type_integer_variant() {
        let ft = FieldType::Integer {
            min: Some(-100),
            max: Some(100),
        };
        assert_eq!(ft, ft.clone());
    }

    #[test]
    fn field_type_simple_variants() {
        assert_eq!(FieldType::Boolean, FieldType::Boolean);
        assert_eq!(FieldType::Timestamp, FieldType::Timestamp);
    }

    #[test]
    fn field_type_enum_variant() {
        let ft = FieldType::Enum {
            one_of: vec!["a".to_string(), "b".to_string()],
        };
        assert_eq!(ft, ft.clone());
    }

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

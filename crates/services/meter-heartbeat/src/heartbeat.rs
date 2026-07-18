//! Turn one raw IoT message (+ its topic) into a liveness "heartbeat" row.
//! NO payload decoding — only the transport envelope (who sent, via which gateway, when).
//! `schematype`/`customerid` are recovered from the topic via a map harvested from the
//! existing per-customer IoT rules (topic → literals), since they aren't derivable from
//! the topic layout (e.g. topic `1095/pulse_v1/#` → schematype `adeunis_pu_v1`).

use serde::Serialize;
use serde_json::Value;

/// topic-pattern (MQTT `/#` = prefix, else exact) → (customerid, schematype).
/// Harvested 2026-07-18 from `aws iot get-topic-rule` across all rules; re-harvest to refresh.
const TOPIC_MAP: &[(&str, &str, &str)] = &[
    ("1095/pulse_v1/#", "1095", "adeunis_pu_v1"),
    ("pulse/123", "123", "adeunis_pu_v1"),
    ("flowiq2200/123", "123", "flowiq2200_v1"),
    ("emu/123", "123", "emu_profes_v1"),
    ("mc603/123", "123", "elvacocmi4_v1"),
    ("iotfabrikken/std_json_v1/#", "iotfabrikken", "std_json_v1"),
    ("iotfabrikken/iotf_json_v1/#", "iotfabrikken", "iotf_json_v1"),
    ("newmqtttest/std_jsonl_v1/#", "newmqtttest", "std_jsonl_v1"),
    ("1234/gwb143_json_v1/#", "1234", "gwb143_json_v1"),
    ("1095/gwb143_json_v1/#", "1095", "gwb143_json_v1"),
    ("mivo/mivo_json_v1/#", "mivo", "mivo_json_v1"),
    ("mivo/https_json_v1/#", "mivo", "https_json_v1"),
    ("brunata_metrona/bluemetering_json_v1/#", "brunata_metrona", "bluemetering_json_v1"),
    ("Dino_Interpreted_data_push_MQTT", "dino", "dino_v1"),
];

fn enrich(topic: &str) -> (Option<&'static str>, Option<&'static str>) {
    for (pat, cust, schem) in TOPIC_MAP {
        let hit = match pat.strip_suffix('#') {
            Some(prefix) => topic.starts_with(prefix), // "a/b/#" -> prefix "a/b/"
            None => topic == *pat,                     // exact
        };
        if hit {
            return (Some(cust), Some(schem));
        }
    }
    (None, None)
}

/// One liveness record. All optional except topic/ingest — an unrecognised transport
/// still yields a heartbeat (someone published on `topic` at `ingest_time`).
#[derive(Debug, Serialize)]
pub struct Heartbeat {
    pub topic: String,
    pub customerid: Option<String>,
    pub schematype: Option<String>,
    pub transport: &'static str,
    pub device_id: Option<String>,
    pub gateway_id: Option<String>,
    pub event_time: Option<String>,
    pub rssi: Option<f64>,
    pub snr: Option<f64>,
    pub fcnt: Option<i64>,
    pub ingest_time: String,
}

/// Build a heartbeat from a parsed message. `ingest_time` is the batch's processing time.
/// Enrichment precedence: the producing rules already stamp `customerid`/`schematype`
/// onto messages on DAQ_INPUT_STREAM, so prefer those; fall back to the topic map (for a
/// topic-based tap where they aren't present).
pub fn from_message(topic: &str, msg: &Value, ingest_time: &str) -> Heartbeat {
    let (map_cust, map_schem) = enrich(topic);
    let customerid = msg
        .get("customerid")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| map_cust.map(str::to_owned));
    let schematype = msg
        .get("schematype")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| map_schem.map(str::to_owned));
    let (transport, device_id, gateway_id, event_time, rssi, snr, fcnt) = liveness(msg);
    Heartbeat {
        topic: topic.to_owned(),
        customerid,
        schematype,
        transport,
        device_id,
        gateway_id,
        event_time,
        rssi,
        snr,
        fcnt,
        ingest_time: ingest_time.to_owned(),
    }
}

type Liveness = (
    &'static str,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<i64>,
);

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(x) => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Pull the (transport, device, gateway, event_time, rssi, snr, fcnt) triple-plus from
/// whichever shape this message is. LoRaWAN (6+ meter types) shares one envelope; the
/// rest are best-effort by known field names; unknown falls back to topic+time only.
fn liveness(m: &Value) -> Liveness {
    // AWS IoT Core for LoRaWAN envelope — the majority, all one shape.
    if let Some(lw) = m.pointer("/WirelessMetadata/LoRaWAN") {
        let gw = lw.pointer("/Gateways/0");
        return (
            "lorawan",
            s(&lw["DevEui"]).or_else(|| s(&m["WirelessDeviceId"])),
            gw.and_then(|g| s(&g["GatewayEui"])),
            s(&lw["Timestamp"]),
            gw.and_then(|g| g["Rssi"].as_f64()),
            gw.and_then(|g| g["Snr"].as_f64()),
            lw["FCnt"].as_i64(),
            );
    }
    // std_json_v1 / std_jsonl_v1 (parsed sensor JSON)
    if let Some(d0) = m.pointer("/data/0") {
        return ("std_json", s(&d0["meterid"]), s(&d0["gatewayid"]), s(&d0["tstamp"]), None, None, None);
    }
    if m.get("id").is_some() && m.get("tstamp").is_some() {
        return ("std_jsonl", s(&m["id"]), None, s(&m["tstamp"]), m["rssi"].as_f64(), None, None);
    }
    // mivo heat JSON
    if let Some(r0) = m.pointer("/Readings/0") {
        return ("mivo", s(&r0["MeterNumber"]), s(&m["UnitSerial"]), s(&r0["Time"]), None, None, None);
    }
    // gwb143 wM-Bus JSON
    if m.get("trbSerial").is_some() {
        return ("gwb143", s(&m["Id"]), s(&m["trbSerial"]), s(m.pointer("/data/0/Timestamp").unwrap_or(&Value::Null)), None, None, None);
    }
    // bluemetering / BRICK4U
    if m.get("meterSerialNumber").is_some() {
        return ("bluemetering", s(&m["meterSerialNumber"]), s(&m["dataSource"]), s(&m["dateTime"]), None, None, None);
    }
    // bestech Modbus
    if m.get("onlinetag").is_some() {
        return ("bestech", s(m.pointer("/payload/0/subDeviceId").unwrap_or(&Value::Null)), s(&m["onlinetag"]), s(&m["time"]), None, None, None);
    }
    // M-Bus datalogger
    if let Some(mt0) = m.pointer("/Meters/0") {
        let dt = match (m.get("Date"), m.get("Time")) {
            (Some(Value::String(d)), Some(Value::String(t))) => Some(format!("{d} {t}")),
            _ => None,
        };
        return ("mbus", s(&mt0["Meter"]), s(&m["Datalogger"]), dt, None, None, None);
    }
    // Unknown transport — still a liveness signal via topic + ingest time.
    ("other", None, None, s(&m["timestamp"]), None, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enrich_prefix_and_exact() {
        assert_eq!(enrich("1095/pulse_v1/abc"), (Some("1095"), Some("adeunis_pu_v1")));
        assert_eq!(enrich("emu/123"), (Some("123"), Some("emu_profes_v1")));
        assert_eq!(enrich("unknown/topic"), (None, None));
    }

    #[test]
    fn lorawan_envelope_extracts() {
        let m = json!({"WirelessDeviceId":"w","WirelessMetadata":{"LoRaWAN":{
            "DevEui":"102ceffffe010ef5","FCnt":35,"Timestamp":"2024-10-30T16:50:01Z",
            "Gateways":[{"GatewayEui":"7076ff006405323d","Rssi":-103,"Snr":0.5}]}}});
        let hb = from_message("emu/123", &m, "2026-07-18T00:00:00Z");
        assert_eq!(hb.transport, "lorawan");
        assert_eq!(hb.device_id.as_deref(), Some("102ceffffe010ef5"));
        assert_eq!(hb.gateway_id.as_deref(), Some("7076ff006405323d"));
        assert_eq!(hb.schematype.as_deref(), Some("emu_profes_v1"));
        assert_eq!(hb.fcnt, Some(35));
    }

    #[test]
    fn std_json_extracts() {
        let m = json!({"data":[{"meterid":"61424","gatewayid":"gw1","tstamp":"2025-05-16 15:18:36"}]});
        let hb = from_message("iotfabrikken/std_json_v1/x", &m, "t");
        assert_eq!(hb.transport, "std_json");
        assert_eq!(hb.device_id.as_deref(), Some("61424"));
    }
}

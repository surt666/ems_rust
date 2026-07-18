//! Encode a batch of heartbeats as a single Parquet object (zstd). One Kinesis batch →
//! one Parquet file, so bigger ESM batches = bigger files (the no-job compaction lever).

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

use crate::heartbeat::Heartbeat;

pub fn encode(rows: &[Heartbeat]) -> anyhow::Result<Vec<u8>> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("topic", DataType::Utf8, false),
        Field::new("customerid", DataType::Utf8, true),
        Field::new("schematype", DataType::Utf8, true),
        Field::new("transport", DataType::Utf8, false),
        Field::new("device_id", DataType::Utf8, true),
        Field::new("gateway_id", DataType::Utf8, true),
        Field::new("event_time", DataType::Utf8, true),
        Field::new("rssi", DataType::Float64, true),
        Field::new("snr", DataType::Float64, true),
        Field::new("fcnt", DataType::Int64, true),
        Field::new("ingest_time", DataType::Utf8, false),
    ]));

    let str_opt = |f: fn(&Heartbeat) -> Option<&str>| -> ArrayRef {
        Arc::new(StringArray::from(rows.iter().map(f).collect::<Vec<_>>()))
    };
    let cols: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.topic.as_str()))),
        str_opt(|r| r.customerid.as_deref()),
        str_opt(|r| r.schematype.as_deref()),
        Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.transport))),
        str_opt(|r| r.device_id.as_deref()),
        str_opt(|r| r.gateway_id.as_deref()),
        str_opt(|r| r.event_time.as_deref()),
        Arc::new(Float64Array::from(rows.iter().map(|r| r.rssi).collect::<Vec<_>>())),
        Arc::new(Float64Array::from(rows.iter().map(|r| r.snr).collect::<Vec<_>>())),
        Arc::new(Int64Array::from(rows.iter().map(|r| r.fcnt).collect::<Vec<_>>())),
        Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.ingest_time.as_str()))),
    ];

    let batch = RecordBatch::try_new(schema.clone(), cols)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
        .build();
    let mut buf = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, schema, Some(props))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(buf)
}

package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3tables"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

const measurementsBucketArn = "arn:aws:s3tables:eu-central-1:891377204778:bucket/measurements"

func NewS3TablesStack(scope constructs.Construct, id string, props *awscdk.StackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, props)

	awss3tables.NewCfnTable(stack, jsii.String("Raw"), &awss3tables.CfnTableProps{
		TableBucketArn:  jsii.String(measurementsBucketArn),
		Namespace:       jsii.String("all"),
		TableName:       jsii.String("raw_data"),
		OpenTableFormat: jsii.String("ICEBERG"),
		IcebergMetadata: rawIcebergMetadata(),
	})

	awss3tables.NewCfnTable(stack, jsii.String("MeterReadings"), &awss3tables.CfnTableProps{
		TableBucketArn:  jsii.String(measurementsBucketArn),
		Namespace:       jsii.String("all"),
		TableName:       jsii.String("logical_meter_data"),
		OpenTableFormat: jsii.String("ICEBERG"),
		IcebergMetadata: meterReadingsIcebergMetadata(),
	})

	awss3tables.NewCfnTable(stack, jsii.String("Hierarchy"), &awss3tables.CfnTableProps{
		TableBucketArn:  jsii.String(measurementsBucketArn),
		Namespace:       jsii.String("all"),
		TableName:       jsii.String("hierarchy"),
		OpenTableFormat: jsii.String("ICEBERG"),
		IcebergMetadata: hierarchyIcebergMetadata(),
	})

	return stack
}

func rawIcebergMetadata() interface{} {
	return map[string]interface{}{
		"icebergSchema": map[string]interface{}{
			"schemaFieldList": []interface{}{
				field("daq_id", "string", true),
				field("timestamp", "timestamptz", true),
				field("value", "double", true),
				field("unit", "string", true),
				field("ingested_time", "timestamptz", true),
			},
		},
		"icebergPartitionSpec": map[string]interface{}{
			"fields": []interface{}{
				partition(2, "month", "timestamp_month"),
				partition(1, "bucket[64]", "daq_id_bucket"),
			},
		},
		"tableProperties": commonTableProperties("daq_id ASC NULLS LAST, timestamp ASC NULLS LAST"),
	}
}

func meterReadingsIcebergMetadata() interface{} {
	return map[string]interface{}{
		"icebergSchema": map[string]interface{}{
			// hn1=partner, hn2=company hard-coded; hn3..hn9 are schema-defined per
			// company (per ems_ocaml hierarchy model). All ids are ints.
			"schemaFieldList": []interface{}{
				field("logical_id", "int", true),
				field("timestamp", "timestamptz", true),
				field("value", "double", true),
				field("unit", "string", true),
				field("ingested_time", "timestamptz", true),
				field("hn1", "int", true),
				field("hn2", "int", true),
				field("hn3", "int", false),
				field("hn4", "int", false),
				field("hn5", "int", false),
				field("hn6", "int", false),
				field("hn7", "int", false),
				field("hn8", "int", false),
				field("hn9", "int", false),
				field("purpose", "string", false),
				field("resample_value", "double", false),
				field("resample_method", "string", false),
				field("resample_timestamp", "timestamp", false),
			},
		},
		"icebergPartitionSpec": map[string]interface{}{
			"fields": []interface{}{
				partition(2, "month", "timestamp_month"),
				partition(7, "bucket[4]", "hn2_bucket"),
			},
		},
		"icebergSortOrder": map[string]interface{}{
			"orderId": 1,
			"fields": []interface{}{
				sortField(7), sortField(8), sortField(9), sortField(1), sortField(2),
			},
		},
		"tableProperties": commonTableProperties(
			"hn2 ASC NULLS LAST, hn3 ASC NULLS LAST, hn4 ASC NULLS LAST, logical_id ASC NULLS LAST, timestamp ASC NULLS LAST",
		),
	}
}

func hierarchyIcebergMetadata() interface{} {
	return map[string]interface{}{
		"icebergSchema": map[string]interface{}{
			"schemaFieldList": []interface{}{
				field("partner_id", "int", true),
				field("company_id", "int", false),
				field("property_id", "int", false),
				field("building_id", "int", false),
				field("area_id", "int", false),
				field("group_id", "int", false),
				field("name", "string", true),
				field("type", "string", true),
				field("ingested_time", "timestamptz", true),
			},
		},
		"icebergPartitionSpec": map[string]interface{}{
			"fields": []interface{}{
				partition(1, "identity", "partner_id"),
				partition(2, "bucket[4]", "company_id_bucket"),
			},
		},
		"icebergSortOrder": map[string]interface{}{
			"orderId": 1,
			"fields": []interface{}{
				sortField(1), sortField(2), sortField(3), sortField(6), sortField(4), sortField(5),
			},
		},
		"tableProperties": commonTableProperties(
			"company_id ASC NULLS LAST, property_id ASC NULLS LAST, building_id ASC NULLS LAST, logical_id ASC NULLS LAST, timestamp ASC NULLS LAST",
		),
	}
}

func field(name, t string, required bool) map[string]interface{} {
	return map[string]interface{}{"name": name, "type": t, "required": required}
}

func partition(sourceId int, transform, name string) map[string]interface{} {
	return map[string]interface{}{"sourceId": sourceId, "transform": transform, "name": name}
}

func sortField(sourceId int) map[string]interface{} {
	return map[string]interface{}{
		"sourceId": sourceId, "transform": "identity", "direction": "asc", "nullOrder": "nulls-last",
	}
}

func commonTableProperties(sortOrder string) map[string]interface{} {
	return map[string]interface{}{
		"write.metadata.delete-after-commit.enabled": "false",
		"write.metadata.previous-versions-max":       "10",
		"write.format.default":                       "parquet",
		"write.parquet.compression-codec":            "zstd",
		"format-version":                             "2",
		"gc.enabled":                                 "false",
		"write.sort-order":                           sortOrder,
		"write.distribution-mode":                    "range",
	}
}

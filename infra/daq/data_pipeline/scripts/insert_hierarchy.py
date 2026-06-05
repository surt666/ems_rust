from datetime import datetime, timezone

from pyspark.context import SparkContext
from awsglue.context import GlueContext
from awsglue.job import Job
from pyspark.sql import Row
from pyspark.sql.types import (
    StructType,
    StructField,
    StringType,
    TimestampType,
    IntegerType,
)

sc = SparkContext()
glueContext = GlueContext(sc)
spark = glueContext.spark_session
job = Job(glueContext)
job.init("InsertHierarchy", {})

spark.conf.set("spark.sql.defaultCatalog", "s3tables")
spark.conf.set("spark.sql.catalog.s3tables", "org.apache.iceberg.spark.SparkCatalog")
spark.conf.set(
    "spark.sql.catalog.s3tables.catalog-impl", "org.apache.iceberg.aws.glue.GlueCatalog"
)
spark.conf.set(
    "spark.sql.catalog.s3tables.glue.id", "891377204778:s3tablescatalog/measurements"
)
spark.conf.set("spark.sql.catalog.s3tables.warehouse", "s3://measurements/warehouse/")

table_ref = "all.hierarchy"

now = datetime.now(timezone.utc)

rows = []

for p in range(1, 6):
    # Partner row
    rows.append(
        Row(
            partner_id=p,
            company_id=None,
            property_id=None,
            building_id=None,
            area_id=None,
            group_id=None,
            name=f"partner_{p}",
            type="partner",
            ingested_time=now,
        )
    )

    for c in range(1, 21):
        # Company row
        rows.append(
            Row(
                partner_id=p,
                company_id=c,
                property_id=None,
                building_id=None,
                area_id=None,
                group_id=None,
                name=f"company_{c}",
                type="company",
                ingested_time=now,
            )
        )

        use_properties = c <= 10
        building_counter = 0
        area_counter = 0

        for slot in range(1, 26):
            if use_properties:
                # Property row
                rows.append(
                    Row(
                        partner_id=p,
                        company_id=c,
                        property_id=slot,
                        building_id=None,
                        area_id=None,
                        group_id=None,
                        name=f"property_{slot}",
                        type="property",
                        ingested_time=now,
                    )
                )

                for b in range(1, 11):
                    building_counter += 1
                    rows.append(
                        Row(
                            partner_id=p,
                            company_id=c,
                            property_id=slot,
                            building_id=b,
                            area_id=None,
                            group_id=None,
                            name=f"building_{b}",
                            type="building",
                            ingested_time=now,
                        )
                    )

                    # 10% of buildings get an area
                    if building_counter % 10 == 0:
                        area_counter += 1
                        rows.append(
                            Row(
                                partner_id=p,
                                company_id=c,
                                property_id=slot,
                                building_id=b,
                                area_id=area_counter,
                                group_id=None,
                                name=f"area_{area_counter}",
                                type="area",
                                ingested_time=now,
                            )
                        )
            else:
                # Group row
                rows.append(
                    Row(
                        partner_id=p,
                        company_id=c,
                        property_id=None,
                        building_id=None,
                        area_id=None,
                        group_id=slot,
                        name=f"group_{slot}",
                        type="group",
                        ingested_time=now,
                    )
                )

                for b in range(1, 11):
                    building_counter += 1
                    rows.append(
                        Row(
                            partner_id=p,
                            company_id=c,
                            property_id=None,
                            building_id=b,
                            area_id=None,
                            group_id=slot,
                            name=f"building_{b}",
                            type="building",
                            ingested_time=now,
                        )
                    )

                    # 10% of buildings get an area
                    if building_counter % 10 == 0:
                        area_counter += 1
                        rows.append(
                            Row(
                                partner_id=p,
                                company_id=c,
                                property_id=None,
                                building_id=b,
                                area_id=area_counter,
                                group_id=slot,
                                name=f"area_{area_counter}",
                                type="area",
                                ingested_time=now,
                            )
                        )

schema = StructType(
    [
        StructField("partner_id", IntegerType(), False),
        StructField("company_id", IntegerType(), True),
        StructField("property_id", IntegerType(), True),
        StructField("building_id", IntegerType(), True),
        StructField("area_id", IntegerType(), True),
        StructField("group_id", IntegerType(), True),
        StructField("name", StringType(), False),
        StructField("type", StringType(), False),
        StructField("ingested_time", TimestampType(), False),
    ]
)

df = spark.createDataFrame(rows, schema)

df = df.orderBy(
    "partner_id", "company_id", "property_id", "building_id", "area_id", "group_id"
)

df.writeTo(table_ref).append()

print(f"Inserted {df.count()} rows into {table_ref}")

job.commit()

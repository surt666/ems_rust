package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

import scala.io.Source
import scala.util.Using

/** Cross-artifact guard: the Flink sink's column list vs the CDK table definition.
  *
  * Renames inside this app are self-consistent by construction — code and fixtures move
  * together — so no ordinary test can tell you the Iceberg table still has the old column.
  * This one reads the Go source that actually creates the table.
  */
class LogicalDataSchemaSpec extends AnyFlatSpec with Matchers:

  /** Field names from the `logical_data` table block of s3tables_stack.go, in order. */
  private def goSchemaFields(): Seq[String] =
    val go = Using.resource(Source.fromFile("../s3tables_stack.go"))(_.mkString)

    val start = go.indexOf("""jsii.String("logical_data")""")
    assert(start > -1, "no logical_data table in s3tables_stack.go")

    val end = go.indexOf("icebergPartitionSpec", start)
    assert(end > start, "logical_data block has no partition spec")

    """field\("([a-z0-9_]+)"""".r
      .findAllMatchIn(go.substring(start, end))
      .map(_.group(1))
      .toSeq

  "the Iceberg table definition" should "declare exactly the columns the sink writes" in {
    goSchemaFields() shouldBe LogicalDataSchema.columns.toSeq
  }

  it should "not still carry the pre-rename column name" in {
    goSchemaFields() should not contain "purpose"
  }

  "the sensor-identity attribute names" should "be snake_case on the wire" in {
    // camelCase here means a bulk rename leaked a Scala field name into a DynamoDB key.
    val attrs = LogicalDataSchema.SensorIdentityAttrs
    val all = Seq(attrs.logicalId, attrs.readingKind, attrs.energyType,
                  attrs.hierarchyPath, attrs.resampleMins)
    all.foreach(a => a should fullyMatch regex "[a-z0-9_]+")
  }

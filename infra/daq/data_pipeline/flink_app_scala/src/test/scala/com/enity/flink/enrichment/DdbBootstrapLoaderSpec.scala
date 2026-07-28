package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers
import software.amazon.awssdk.services.dynamodb.model.AttributeValue

class DdbBootstrapLoaderSpec extends AnyFlatSpec with Matchers {

  "DdbBootstrapLoader.zeroPad" should "pad single digit to 5 chars" in {
    DdbBootstrapLoader.zeroPad(1) shouldBe "00001"
  }

  it should "pad large number" in {
    DdbBootstrapLoader.zeroPad(19999) shouldBe "19999"
  }

  it should "pad zero" in {
    DdbBootstrapLoader.zeroPad(0) shouldBe "00000"
  }

  "DdbBootstrapLoader.partitionKey" should "compute consistent partition" in {
    val daqId = "daq:std_json_v1:customer:meter:sensor"
    val pk = DdbBootstrapLoader.partitionKey(daqId)
    pk.length shouldBe 5
    pk.toInt should be >= 0
    pk.toInt should be < 20000
  }

  it should "produce same result for same input" in {
    val daqId = "daq:test:cust:m1:s1"
    DdbBootstrapLoader.partitionKey(daqId) shouldBe DdbBootstrapLoader.partitionKey(daqId)
  }

  "DdbBootstrapLoader.parseDdbItem" should "parse a DynamoDB item map" in {
    val item = new java.util.HashMap[String, AttributeValue]()
    item.put("pk", AttributeValue.builder().s("04821").build())
    item.put("sk", AttributeValue.builder().s("daq:std:cust:m1:temp").build())
    item.put("logical_id", AttributeValue.builder().n("101").build())
    item.put("reading_kind", AttributeValue.builder().s("gauge").build())
    item.put("hierarchy_path", AttributeValue.builder().s("HN0#root|HN1#1|HN2#2|HN3#8|HN4#3").build())

    val (daqId, mapping) = DdbBootstrapLoader.parseDdbItem(item)
    daqId shouldBe "daq:std:cust:m1:temp"
    mapping.logicalId shouldBe 101
    mapping.readingKind shouldBe "gauge"
    mapping.hn1 shouldBe 1
    mapping.hn2 shouldBe 2
    mapping.hn3 shouldBe java.lang.Integer.valueOf(8)
    mapping.hn4 shouldBe java.lang.Integer.valueOf(3)
    mapping.resampleMinutes shouldBe null
  }

  it should "parse resample_minutes and energyType when present" in {
    val item = new java.util.HashMap[String, AttributeValue]()
    item.put("pk", AttributeValue.builder().s("00001").build())
    item.put("sk", AttributeValue.builder().s("daq:std:cust:m2:energy").build())
    item.put("logical_id", AttributeValue.builder().n("42").build())
    item.put("reading_kind", AttributeValue.builder().s("counter").build())
    item.put("hierarchy_path", AttributeValue.builder().s("HN0#root|HN1#1|HN2#2|HN3#3").build())
    item.put("resample_minutes", AttributeValue.builder().n("15").build())
    item.put("energy_type", AttributeValue.builder().s("electricity").build())

    val (_, mapping) = DdbBootstrapLoader.parseDdbItem(item)
    mapping.resampleMinutes shouldBe java.lang.Integer.valueOf(15)
    mapping.energyType shouldBe "electricity"
  }

}

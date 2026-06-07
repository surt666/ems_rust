package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers
import java.nio.charset.StandardCharsets

class DdbStreamDeserializerSpec extends AnyFlatSpec with Matchers {

  private val deserializer = new DdbStreamDeserializer()

  private def makeJson(eventName: String, image: String, imageKey: String = "NewImage"): Array[Byte] =
    s"""{
       |  "eventName": "$eventName",
       |  "dynamodb": {
       |    "$imageKey": $image
       |  }
       |}""".stripMargin.getBytes(StandardCharsets.UTF_8)

  "DdbStreamDeserializer" should "parse INSERT event" in {
    val image = s"""{
      "pk": {"S": "04821"},
      "sk": {"S": "daq:std_json_v1:cust:meter1:temp"},
      "logical_id": {"N": "101"},
      "meter_type": {"S": "gauge"},
      "hierarchy_path": {"S": "HN0#root|HN1#1|HN2#2|HN3#8|HN4#3"}
    }"""
    val result = deserializer.deserialize(makeJson("INSERT", image))

    result.eventType shouldBe "INSERT"
    result.daqId shouldBe "daq:std_json_v1:cust:meter1:temp"
    result.mapping shouldBe defined
    val m = result.mapping.get
    m.logicalId shouldBe 101
    m.meterType shouldBe "gauge"
    m.hn1 shouldBe 1
    m.hn2 shouldBe 2
    m.hn3 shouldBe java.lang.Integer.valueOf(8)
    m.hn4 shouldBe java.lang.Integer.valueOf(3)
  }

  it should "parse MODIFY event" in {
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:emu:cust:m2:energy"},
      "logical_id": {"N": "42"},
      "meter_type": {"S": "counter"},
      "hierarchy_path": {"S": "HN0#root|HN1#5|HN2#10|HN3#3|HN4#7"}
    }"""
    val result = deserializer.deserialize(makeJson("MODIFY", image))

    result.eventType shouldBe "MODIFY"
    result.daqId shouldBe "daq:emu:cust:m2:energy"
    result.mapping.get.meterType shouldBe "counter"
    result.mapping.get.hn3 shouldBe java.lang.Integer.valueOf(3)
    result.mapping.get.hn4 shouldBe java.lang.Integer.valueOf(7)
  }

  it should "parse REMOVE event using OldImage" in {
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:std:cust:m3:temp"}
    }"""
    val result = deserializer.deserialize(makeJson("REMOVE", image, "OldImage"))

    result.eventType shouldBe "REMOVE"
    result.daqId shouldBe "daq:std:cust:m3:temp"
    result.mapping shouldBe None
  }

  it should "parse resample_minutes and purpose when present" in {
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:std:cust:m1:energy"},
      "logical_id": {"N": "12"},
      "meter_type": {"S": "counter"},
      "hierarchy_path": {"S": "HN0#root|HN1#1|HN2#2|HN3#3"},
      "resample_minutes": {"N": "15"},
      "purpose": {"S": "main meter"}
    }"""
    val result = deserializer.deserialize(makeJson("INSERT", image))
    result.mapping.get.resampleMinutes shouldBe java.lang.Integer.valueOf(15)
    result.mapping.get.purpose shouldBe "main meter"
  }


  it should "set optional fields empty/null when not present" in {
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:std:cust:m1:temp"},
      "logical_id": {"N": "9"},
      "meter_type": {"S": "gauge"},
      "hierarchy_path": {"S": "HN0#root|HN1#1|HN2#2|HN3#3"}
    }"""
    val result = deserializer.deserialize(makeJson("INSERT", image))
    result.mapping.get.resampleMinutes shouldBe null
    result.mapping.get.purpose shouldBe ""
  }
}

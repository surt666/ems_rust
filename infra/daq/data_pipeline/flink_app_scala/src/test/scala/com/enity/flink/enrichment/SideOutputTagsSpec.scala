package com.enity.flink.enrichment

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class SideOutputTagsSpec extends AnyFlatSpec with Matchers {
  private val mapper = new ObjectMapper()
  mapper.registerModule(DefaultScalaModule)

  "ErrorRecord" should "serialize to JSON with all fields" in {
    val error = ErrorRecord(
      errorType = "parse_error",
      timestamp = "2026-03-27T18:00:00Z",
      daqId = "daq:std_json_v1:customer:meter:sensor",
      payload = """{"bad":"json""",
      error = "Unexpected end of input"
    )
    val json = mapper.writeValueAsString(error)
    val parsed = mapper.readValue(json, classOf[Map[String, Any]])

    parsed("type") shouldBe "parse_error"
    parsed("daq_id") shouldBe "daq:std_json_v1:customer:meter:sensor"
    parsed("error") shouldBe "Unexpected end of input"
  }

  "ErrorRecord" should "use 'type' as the JSON field name, not 'errorType'" in {
    val error = ErrorRecord("dead_letter", "2026-03-27T18:00:00Z", "daq:x", "{}", "no mapping")
    val json = mapper.writeValueAsString(error)
    json should include("\"type\"")
    json should not include("\"errorType\"")
  }

  "SideOutputTags" should "have distinct tag IDs" in {
    SideOutputTags.PARSE_ERROR.getId shouldBe "parse-error"
    SideOutputTags.DEAD_LETTER.getId shouldBe "dead-letter"
    SideOutputTags.ANOMALY.getId shouldBe "anomaly"
    SideOutputTags.LATE_ARRIVAL.getId shouldBe "late-arrival"
  }
}

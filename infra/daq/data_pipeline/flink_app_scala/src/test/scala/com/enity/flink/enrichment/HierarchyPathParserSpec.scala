package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class HierarchyPathParserSpec extends AnyFlatSpec with Matchers {

  "HierarchyPathParser" should "parse partner+company minimal path" in {
    val r = HierarchyPathParser.parse("HN0#root|HN1#1|HN2#2")
    r.hn1 shouldBe 1
    r.hn2 shouldBe 2
    r.hn3 shouldBe null
    r.hn9 shouldBe null
  }

  it should "parse partner→company→hn3→hn4 path" in {
    val r = HierarchyPathParser.parse("HN0#root|HN1#1|HN2#2|HN3#5|HN4#8")
    r.hn1 shouldBe 1
    r.hn2 shouldBe 2
    r.hn3 shouldBe java.lang.Integer.valueOf(5)
    r.hn4 shouldBe java.lang.Integer.valueOf(8)
    r.hn5 shouldBe null
  }

  it should "skip non-contiguous depths if absent" in {
    // hn2 → hn4 (hn3 skipped per company schema)
    val r = HierarchyPathParser.parse("HN0#root|HN1#1|HN2#2|HN4#8")
    r.hn1 shouldBe 1
    r.hn2 shouldBe 2
    r.hn3 shouldBe null
    r.hn4 shouldBe java.lang.Integer.valueOf(8)
  }

  it should "parse path without root prefix" in {
    val r = HierarchyPathParser.parse("HN1#10|HN2#20|HN3#30")
    r.hn1 shouldBe 10
    r.hn2 shouldBe 20
    r.hn3 shouldBe java.lang.Integer.valueOf(30)
  }

  it should "handle large IDs" in {
    val r = HierarchyPathParser.parse("HN0#root|HN1#9999|HN2#12345|HN3#67890")
    r.hn1 shouldBe 9999
    r.hn2 shouldBe 12345
    r.hn3 shouldBe java.lang.Integer.valueOf(67890)
  }

  it should "fill all hn3..hn9 levels" in {
    val r = HierarchyPathParser.parse(
      "HN0#root|HN1#1|HN2#2|HN3#3|HN4#4|HN5#5|HN6#6|HN7#7|HN8#8|HN9#9"
    )
    r.hn1 shouldBe 1; r.hn2 shouldBe 2
    r.hn3 shouldBe java.lang.Integer.valueOf(3)
    r.hn4 shouldBe java.lang.Integer.valueOf(4)
    r.hn5 shouldBe java.lang.Integer.valueOf(5)
    r.hn6 shouldBe java.lang.Integer.valueOf(6)
    r.hn7 shouldBe java.lang.Integer.valueOf(7)
    r.hn8 shouldBe java.lang.Integer.valueOf(8)
    r.hn9 shouldBe java.lang.Integer.valueOf(9)
  }

  it should "throw on missing HN1" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("HN0#root|HN2#2")
    }
  }

  it should "throw on missing HN2" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("HN0#root|HN1#1")
    }
  }

  it should "throw on empty string" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("")
    }
  }

  it should "throw on unrecognized segment" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("HN0#root|HN1#1|HN2#2|XX#9")
    }
  }
}

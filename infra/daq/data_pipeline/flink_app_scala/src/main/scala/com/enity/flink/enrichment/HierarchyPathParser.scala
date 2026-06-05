package com.enity.flink.enrichment

/** Hierarchy levels hn1..hn9. hn1 always = partner, hn2 always = company; hn3..hn9 are
  * schema-defined per company (see ems_ocaml hierarchy model). java.lang.Integer for
  * nullable optional levels — Flink Kryo corrupts Scala Option[Int] across operator boundaries. */
case class HierarchyIds(
  hn1: Int,
  hn2: Int,
  hn3: java.lang.Integer,
  hn4: java.lang.Integer,
  hn5: java.lang.Integer,
  hn6: java.lang.Integer,
  hn7: java.lang.Integer,
  hn8: java.lang.Integer,
  hn9: java.lang.Integer
)

object HierarchyPathParser:

  /** Parse an OCaml-style hierarchy path into level ids.
    *
    * Input shape: `HN0#root|HN1#<int>|HN2#<int>|...` — pipe-separated `HN<n>#<id>` segments
    * starting at the root (`HN0#root`, ignored) and descending. The trailing `S#<id>` segment
    * (the sensor itself) must NOT be present in the meter-identity `hierarchy_path` value;
    * the sensor id is the table's `logical_id`.
    *
    * hn1 (partner) and hn2 (company) are required; hn3..hn9 are filled if present, else null. */
  def parse(path: String): HierarchyIds =
    require(path.nonEmpty, "Hierarchy path must not be empty")

    val segments = path.split('|').filter(_.nonEmpty)
    var hn1: Option[Int] = None
    var hn2: Option[Int] = None
    val rest = scala.collection.mutable.Map.empty[Int, java.lang.Integer]

    segments.foreach { seg =>
      if seg == "HN0#root" then ()
      else if seg.length < 4 || !seg.startsWith("HN") || seg.charAt(3) != '#' then
        throw IllegalArgumentException(s"Unrecognized hierarchy segment: $seg")
      else
        val depthChar = seg.charAt(2)
        if depthChar < '1' || depthChar > '9' then
          throw IllegalArgumentException(s"Unsupported hierarchy level in segment: $seg")
        val depth = depthChar - '0'
        val idStr = seg.substring(4)
        val id = idStr.toInt
        depth match
          case 1 => hn1 = Some(id)
          case 2 => hn2 = Some(id)
          case n => rest(n) = Integer.valueOf(id)
    }

    require(hn1.isDefined, s"Hierarchy path missing HN1 (partner): $path")
    require(hn2.isDefined, s"Hierarchy path missing HN2 (company): $path")

    HierarchyIds(
      hn1 = hn1.get,
      hn2 = hn2.get,
      hn3 = rest.getOrElse(3, null),
      hn4 = rest.getOrElse(4, null),
      hn5 = rest.getOrElse(5, null),
      hn6 = rest.getOrElse(6, null),
      hn7 = rest.getOrElse(7, null),
      hn8 = rest.getOrElse(8, null),
      hn9 = rest.getOrElse(9, null)
    )

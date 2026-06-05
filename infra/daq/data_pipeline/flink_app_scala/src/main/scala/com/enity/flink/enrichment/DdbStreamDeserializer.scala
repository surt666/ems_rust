package com.enity.flink.enrichment

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule
import org.apache.flink.api.common.serialization.DeserializationSchema
import org.apache.flink.api.common.typeinfo.TypeInformation
import org.slf4j.LoggerFactory

/** Deserializes DynamoDB Streams JSON records (via Kinesis adapter) into IdMappingChange. */
class DdbStreamDeserializer extends DeserializationSchema[IdMappingChange]:
  @transient private lazy val logger = LoggerFactory.getLogger(getClass)
  @transient private lazy val mapper =
    val m = new ObjectMapper()
    m.registerModule(DefaultScalaModule)
    m

  override def deserialize(message: Array[Byte]): IdMappingChange =
    val record = mapper.readValue(message, classOf[Map[String, Any]])
    val eventName = record("eventName").toString
    val dynamodb = record("dynamodb").asInstanceOf[Map[String, Any]]

    eventName match
      case "REMOVE" =>
        val oldImage = dynamodb("OldImage").asInstanceOf[Map[String, Any]]
        val daqId = extractString(oldImage, "sk")
        IdMappingChange("REMOVE", daqId, None)

      case "INSERT" | "MODIFY" =>
        val newImage = dynamodb("NewImage").asInstanceOf[Map[String, Any]]
        val daqId = extractString(newImage, "sk")
        val logicalId = extractRequiredNumber(newImage, "logical_id").toInt
        val meterType = extractString(newImage, "meter_type")
        val hierarchyPath = extractString(newImage, "hierarchy_path")
        val ids = HierarchyPathParser.parse(hierarchyPath)
        val purpose = extractOptionalString(newImage, "purpose").getOrElse("")
        val binning: java.lang.Integer = extractOptionalNumber(newImage, "binning")

        val mapping = MeterMapping(
          logicalId = logicalId,
          meterType = meterType,
          hn1 = ids.hn1, hn2 = ids.hn2,
          hn3 = ids.hn3, hn4 = ids.hn4, hn5 = ids.hn5,
          hn6 = ids.hn6, hn7 = ids.hn7, hn8 = ids.hn8, hn9 = ids.hn9,
          purpose = purpose,
          binning = binning
        )
        IdMappingChange(eventName, daqId, Some(mapping))

      case other =>
        logger.warn(s"Unknown DDB Streams event type: $other")
        IdMappingChange(other, "", None)

  override def isEndOfStream(nextElement: IdMappingChange): Boolean = false

  override def getProducedType: TypeInformation[IdMappingChange] =
    TypeInformation.of(classOf[IdMappingChange])

  private def extractString(image: Map[String, Any], field: String): String =
    image(field).asInstanceOf[Map[String, Any]]("S").toString

  private def extractOptionalString(image: Map[String, Any], field: String): Option[String] =
    image.get(field) match
      case Some(m: Map[?, ?]) =>
        m.asInstanceOf[Map[String, Any]].get("S").map(_.toString)
      case _ => None

  private def extractRequiredNumber(image: Map[String, Any], field: String): String =
    image(field).asInstanceOf[Map[String, Any]]("N").toString

  private def extractOptionalNumber(image: Map[String, Any], field: String): java.lang.Integer =
    image.get(field) match
      case Some(m: Map[?, ?]) =>
        m.asInstanceOf[Map[String, Any]].get("N") match
          case Some(n) => Integer.valueOf(n.toString.toDouble.toInt)
          case None    => null
      case _ => null

name := "flink-app-scala"

version := "0.1.0"

scalaVersion := "3.3.4"

organization := "com.enity"

val flinkVersion = "1.20.3"
val jacksonVersion = "2.15.3"
val icebergVersion = "1.7.1"

libraryDependencies ++= Seq(
  // Flink core dependencies (Java API - Scala wrappers removed for Scala 3 compatibility)
  "org.apache.flink" % "flink-streaming-java" % flinkVersion % "provided",
  "org.apache.flink" % "flink-clients" % flinkVersion % "provided",
  "org.apache.flink" % "flink-connector-base" % flinkVersion,
  "org.apache.flink" % "flink-table-api-java" % flinkVersion % "provided",

  // Flink Kinesis connector (source)
  "org.apache.flink" % "flink-connector-kinesis" % "5.0.0-1.20",

  // Iceberg + S3 Tables sink
  "org.apache.iceberg" % "iceberg-flink-runtime-1.20" % icebergVersion,
  ("software.amazon.s3tables" % "s3-tables-catalog-for-iceberg" % "0.1.8")
    .excludeAll(ExclusionRule(organization = "org.apache.iceberg")),

  // Hadoop (required by Iceberg CatalogLoader)
  "org.apache.hadoop" % "hadoop-common" % "3.4.1",

  // JSON processing
  "com.fasterxml.jackson.core" % "jackson-core" % jacksonVersion,
  "com.fasterxml.jackson.core" % "jackson-databind" % jacksonVersion,
  "com.fasterxml.jackson.module" %% "jackson-module-scala" % jacksonVersion,

  // Logging
  "org.slf4j" % "slf4j-api" % "2.0.9",
  "ch.qos.logback" % "logback-classic" % "1.4.11" % "runtime",

  // Testing
  "org.scalatest" %% "scalatest" % "3.2.17" % Test,
  "org.scalatestplus" %% "mockito-4-11" % "3.2.17.0" % Test,

  // AWS SDK for DynamoDB bootstrap scan
  "software.amazon.awssdk" % "dynamodb" % "2.25.0",

  // Flink test utils for harness-based unit tests
  "org.apache.flink" % "flink-test-utils" % flinkVersion % Test,
  "org.apache.flink" % "flink-streaming-java" % flinkVersion % Test classifier "tests",
  "org.apache.flink" % "flink-runtime" % flinkVersion % Test classifier "tests"
)

// Assembly settings for creating fat JAR
assembly / assemblyMergeStrategy := {
  case PathList("META-INF", xs @ _*) =>
    xs match {
      case "MANIFEST.MF" :: Nil => MergeStrategy.discard
      case "services" :: _ => MergeStrategy.concat
      case _ => MergeStrategy.discard
    }
  case "reference.conf" => MergeStrategy.concat
  case x if x.endsWith(".proto") => MergeStrategy.first
  case x if x.contains("module-info") => MergeStrategy.discard
  case _ => MergeStrategy.first
}

assembly / assemblyJarName := s"${name.value}-${version.value}.jar"

// Compiler options
scalacOptions ++= Seq(
  "-encoding", "UTF-8",
  "-deprecation",
  "-feature",
  "-unchecked"
)

// Java compatibility
javacOptions ++= Seq("-source", "1.8", "-target", "1.8")

// JVM flags required by Flink's Kryo serializer on Java 17+
Test / javaOptions ++= Seq(
  "--add-opens=java.base/java.util=ALL-UNNAMED",
  "--add-opens=java.base/java.lang=ALL-UNNAMED",
  "--add-opens=java.base/java.lang.reflect=ALL-UNNAMED",
  "--add-opens=java.base/java.io=ALL-UNNAMED",
  "--add-opens=java.base/java.net=ALL-UNNAMED",
  "--add-opens=java.base/java.time=ALL-UNNAMED"
)
Test / fork := true

package com.enity.flink.scenarios

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule

import java.io.{File, PrintWriter}
import java.time.Instant

case class ScenarioEntry(
  name: String,
  status: String,    // "PASS" or "FAIL"
  durationMs: Long,
  error: Option[String] = None,
  expected: Option[String] = None,
  actual: Option[String] = None,
  inputs: Option[Map[String, Any]] = None
)

case class ReportSummary(total: Int, passed: Int, failed: Int)

case class FullReport(
  layer: String,
  timestamp: String,
  durationMs: Long,
  summary: ReportSummary,
  scenarios: List[ScenarioEntry]
)

object ScenarioReport:

  private val mapper = new ObjectMapper()
  mapper.registerModule(DefaultScalaModule)
  mapper.writerWithDefaultPrettyPrinter()

  def printConsole(report: FullReport): Unit =
    val header = s"=== Scenario Test Report ==="
    val stats = s"Layer: ${report.layer} | ${report.summary.total} scenarios | " +
      s"${report.summary.passed} passed | ${report.summary.failed} failed | " +
      f"${report.durationMs / 1000.0}%.1fs"

    println(header)
    println(stats)
    println()

    report.scenarios.foreach { s =>
      val tag = if s.status == "PASS" then "[PASS]" else "[FAIL]"
      val line = f"  $tag%-6s ${s.name}%-50s (${s.durationMs / 1000.0}%.1fs)"
      println(line)
    }

    val failures = report.scenarios.filter(_.status == "FAIL")
    if failures.nonEmpty then
      println()
      println("--- FAILURE DETAILS ---")
      println()
      failures.foreach { f =>
        println(s"${f.name}:")
        f.expected.foreach(e => println(s"  Expected: $e"))
        f.actual.foreach(a => println(s"  Actual:   $a"))
        f.error.foreach(e => println(s"  Error:    $e"))
        println()
      }

  def writeJson(report: FullReport, outputDir: String = "target/test-reports"): String =
    val dir = new File(outputDir)
    if !dir.exists() then dir.mkdirs()
    val filename = s"scenario-${report.layer}-${Instant.now().toString.replace(":", "-")}.json"
    val file = new File(dir, filename)
    val writer = new PrintWriter(file)
    try
      writer.write(mapper.writerWithDefaultPrettyPrinter().writeValueAsString(report))
    finally
      writer.close()
    file.getAbsolutePath

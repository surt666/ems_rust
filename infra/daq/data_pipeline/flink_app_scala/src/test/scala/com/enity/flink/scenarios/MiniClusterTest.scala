package com.enity.flink.scenarios

import org.scalatest.{BeforeAndAfterEach, Suite}

trait MiniClusterTest extends BeforeAndAfterEach { self: Suite =>
  override def beforeEach(): Unit =
    super.beforeEach()
    CollectSinks.clear()
}

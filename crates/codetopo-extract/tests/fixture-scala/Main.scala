package quarry

import quarry.core.Engine
import quarry.shapes.Circle

object Main {
  def main(args: Array[String]): Unit = {
    val engine = new Engine()
    val wheel = new Circle(2.0)
    println(engine.render(wheel))
  }
}

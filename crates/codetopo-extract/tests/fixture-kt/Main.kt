package quarry

import quarry.core.Engine
import quarry.shapes.Circle

fun main() {
    val engine = Engine(Circle(2.0))
    println(engine.render())
}

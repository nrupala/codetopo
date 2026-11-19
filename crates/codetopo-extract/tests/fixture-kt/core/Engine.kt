package quarry.core

import quarry.shapes.Circle
import quarry.shapes.Shape

const val DEFAULT_SIZE = 10

var counter = 0

class Engine(val shape: Shape) {
    var size: Int = DEFAULT_SIZE

    fun render(): Double {
        counter += 1
        return shape.area()
    }
}

fun buildEngine(): Engine {
    return Engine(Circle(1.0))
}

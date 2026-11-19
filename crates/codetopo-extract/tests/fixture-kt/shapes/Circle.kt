package quarry.shapes

import kotlin.math.PI

class Circle(val radius: Double) : Shape {
    override fun area(): Double {
        return PI * radius * radius
    }
}

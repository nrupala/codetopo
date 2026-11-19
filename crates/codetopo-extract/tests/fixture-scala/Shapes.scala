package quarry.shapes

trait Shape {
  def area: Double
}

class Circle(val radius: Double) extends Shape {
  def area: Double = 3.141592653589793 * radius * radius
}

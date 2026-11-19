package shapes

type Circle struct {
	Radius float64
}

func NewCircle(radius float64) Circle {
	return Circle{Radius: radius}
}

func (c Circle) Area() float64 {
	return 3.141592653589793 * c.Radius * c.Radius
}

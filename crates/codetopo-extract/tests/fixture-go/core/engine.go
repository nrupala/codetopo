package core

import "quarry/shapes"

const MaxRenders = 4

var Count int

type Engine struct {
	Wheel   shapes.Circle
	renders int
}

func (e *Engine) Render() float64 {
	e.renders++
	Count++
	return e.Wheel.Area()
}

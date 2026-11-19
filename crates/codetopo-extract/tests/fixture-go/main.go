package main

import (
	"fmt"

	"quarry/core"
	"quarry/shapes"
)

func main() {
	engine := core.Engine{Wheel: shapes.NewCircle(2.0)}
	area := engine.Render()
	fmt.Println(area)
}

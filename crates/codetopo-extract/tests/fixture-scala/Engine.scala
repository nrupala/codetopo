package quarry.core

import quarry.shapes.Circle

class Engine {
  private var renders = 0

  def render(wheel: Circle): Double = {
    renders += 1
    wheel.area()
  }

  def renderCount: Int = renders
}

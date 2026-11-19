require_relative "../lib/circle"

module Quarry
  class Engine
    MAX_RENDERS = 4

    def initialize
      @renders = 0
    end

    def render(wheel)
      @renders += 1
      wheel.area
    end
  end
end

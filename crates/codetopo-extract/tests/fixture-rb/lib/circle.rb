require_relative "shape"

module Quarry
  class Circle < Shape
    attr_reader :radius

    def initialize(radius)
      @radius = radius
    end

    def area
      3.141592653589793 * @radius * @radius
    end
  end
end

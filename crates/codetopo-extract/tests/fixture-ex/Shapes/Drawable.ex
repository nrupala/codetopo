defprotocol Quarry.Shapes.Drawable do
  def draw(shape)
end

defimpl Quarry.Shapes.Drawable, for: Quarry.Shapes.Circle do
  def draw(circle) do
    IO.inspect(circle)
  end
end

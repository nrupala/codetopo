defmodule Quarry.Shapes.Circle do
  @behaviour Quarry.Shapes.Shape

  defstruct radius: 0.0

  @pi 3.141592653589793

  def new(radius) do
    %__MODULE__{radius: radius}
  end

  def area(%__MODULE__{radius: radius}) do
    @pi * radius * radius
  end

  defp scale(radius, factor) do
    radius * factor
  end
end

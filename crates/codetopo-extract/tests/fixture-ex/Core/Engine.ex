defmodule Quarry.Core.Engine do
  use GenServer

  alias Quarry.Shapes.Circle
  alias Quarry.Shapes.Drawable
  import Quarry.Shapes.Circle, only: [area: 1]
  require Logger

  defstruct renders: 0

  def new do
    %__MODULE__{renders: 0}
  end

  def render(engine, wheel) do
    if engine && wheel do
      Logger.info("again")
    end

    area(wheel) + Drawable.draw(wheel)
  end
end

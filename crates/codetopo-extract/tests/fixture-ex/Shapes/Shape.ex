defmodule Quarry.Shapes.Shape do
  @callback area(term()) :: float()
end

module Quarry.Core.Engine where

import Data.Maybe (fromMaybe)
import Quarry.Shapes (Circle, Shape, area, describe, scaleFactor, unitCircle)
import qualified Quarry.Shapes as Shapes

data EngineState = EngineState { renders :: Int }

render :: EngineState -> Circle -> Double
render state wheel =
  let scaled = area wheel * scaleFactor
  in if scaled > 0.0 && renders state > 0
       then scaled
       else area unitCircle

renderCount :: EngineState -> Maybe Int -> Int
renderCount state override = fromMaybe (renders state) override

describeAll :: [Circle] -> [String]
describeAll shapes = case shapes of
  [] -> []
  (c : rest) -> describe c : describeAll rest

fallbackArea :: Double
fallbackArea = Shapes.area unitCircle

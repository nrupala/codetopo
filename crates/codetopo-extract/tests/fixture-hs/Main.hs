module Main where

import Quarry.Core.Engine (EngineState (..), describeAll, render, renderCount)
import Quarry.Shapes (Circle (..), unitCircle)

main :: IO ()
main = do
  let wheel = unitCircle
      state = EngineState 0
  print (render state wheel)
  print (renderCount state Nothing)
  print (describeAll [wheel, unitCircle])

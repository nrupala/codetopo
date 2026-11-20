module Quarry.Shapes where

import Data.Maybe (fromMaybe)

class Shape a where
  area :: a -> Double
  describe :: a -> String
  describe s = "a shape"

data Circle = Circle { radius :: Double }
              deriving (Show, Eq)

newtype Radius = Radius Double

type RadiusAlias = Double

instance Shape Circle where
  area (Circle r) = 3.141592653589793 * r * r
  describe c = "circle"

unitCircle :: Circle
unitCircle = Circle 1.0

scaleFactor :: Double
scaleFactor = 2.0

defaultRadius :: Radius
defaultRadius = Radius 1.0

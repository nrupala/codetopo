"""Shapes for the quarry fixture: a base/derived pair and a dataclass."""

import math
from dataclasses import dataclass


class Shape:
    def area(self):
        return 0


class Circle(Shape):
    def __init__(self, radius):
        self.radius = radius

    def area(self):
        return math.pi * self.radius**2


@dataclass
class Point:
    x: float
    y: float

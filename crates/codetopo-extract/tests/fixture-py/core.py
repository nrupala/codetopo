"""Core of the quarry fixture: a Protocol, an engine, a property pair."""

import os
from typing import Protocol

from quarry.shapes import Circle, Shape

DEFAULT_SIZE = 10


class Drawable(Protocol):
    def draw(self) -> None: ...


class Engine:
    def __init__(self, size=DEFAULT_SIZE):
        self.size = size

    @property
    def size(self):
        return self._size

    @size.setter
    def size(self, value):
        self._size = value

    def render(self, shape):
        return shape.draw()

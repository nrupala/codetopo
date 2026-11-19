<?php

namespace Quarry\Shapes;

class Circle implements Shape
{
    private float $radius;

    public function __construct(float $radius)
    {
        $this->radius = $radius;
    }

    public function area(): float
    {
        return 3.141592653589793 * $this->radius * $this->radius;
    }
}

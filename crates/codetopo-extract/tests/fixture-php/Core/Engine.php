<?php

namespace Quarry\Core;

use Quarry\Shapes\Circle;

class Engine
{
    private int $renders = 0;

    public function render(Circle $wheel): float
    {
        $this->renders += 1;
        return $wheel->area();
    }

    public function renderCount(): int
    {
        return $this->renders;
    }
}

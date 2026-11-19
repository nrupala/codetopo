<?php

use Quarry\Core\Engine;
use Quarry\Shapes\Circle;

require_once 'Core/Engine.php';

$engine = new Engine();
$wheel = new Circle(2.0);
echo $engine->render($wheel);

using System;
using Quarry.Core;
using Quarry.Shapes;

var engine = new Engine();
var wheel = new Circle(2.0);
Console.WriteLine(engine.Render(wheel));

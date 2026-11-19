using System.Collections.Generic;
using Quarry.Shapes;

namespace Quarry.Core
{
    public class Engine
    {
        private int _renders;

        public double Render(Circle wheel)
        {
            _renders += 1;
            return wheel.Area();
        }

        public int RenderCount()
        {
            return _renders;
        }
    }
}

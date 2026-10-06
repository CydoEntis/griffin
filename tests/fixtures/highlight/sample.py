# A sample for the highlight tests.
import functools


class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y


@functools.cache
def distance(a: Point, b: Point) -> float:
    return abs(a.x - b.x) * 2.5


origin = Point(0, 0)
print(f"origin is {distance(origin, origin)} away")

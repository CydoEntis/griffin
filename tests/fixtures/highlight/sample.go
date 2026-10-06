// A sample for the highlight tests.
package main

import "fmt"

type Point struct {
	X int
	Y int
}

func distance(a Point, b Point) float64 {
	dx := float64(a.X - b.X)
	return dx * 2.5
}

func main() {
	origin := Point{X: 0, Y: 0}
	fmt.Println("origin", distance(origin, origin))
}

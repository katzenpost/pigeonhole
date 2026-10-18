package thinclient

import "testing"

func TestMostCommon(t *testing.T) {
	for _, c := range []struct {
		in   []uint64
		want uint64
	}{{[]uint64{5}, 5}, {[]uint64{7, 6, 7}, 7}, {[]uint64{3, 4, 4, 3, 4}, 4}} {
		if got, err := mostCommon(c.in); err != nil || got != c.want {
			t.Errorf("%v: %d %v", c.in, got, err)
		}
	}
	if _, err := mostCommon(nil); err == nil {
		t.Error("no epochs accepted")
	}
}

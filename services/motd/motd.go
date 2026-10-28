// Package motd is the message of the day service, a minimal pigeonhole service. A client dials
// it and sends Requests over the connection; the service answers each with the current message.
// The client side is clients/motd.
package motd

import (
	"context"
	"time"

	ps "github.com/katzenpost/katzenpost/pigeonservice"
)

// Request asks for the message of the day.
type Request struct{}

// Reply carries the message of the day.
type Reply struct {
	// Message is the message of the day.
	Message string
	// Day is the date it is the message of, as YYYY-MM-DD in UTC.
	Day string
}

// Source returns the current message of the day.
type Source func() (string, error)

// Serve answers one client's requests until the session ends.
func Serve(ctx context.Context, s *ps.Session, source Source) error {
	for {
		raw, err := s.Recv(ctx)
		if err != nil {
			return err
		}
		var req Request
		if err := ps.Unmarshal(raw, &req); err != nil {
			continue
		}
		msg, err := source()
		if err != nil {
			return err
		}
		reply, err := ps.Marshal(&Reply{Message: msg, Day: time.Now().UTC().Format(time.DateOnly)})
		if err != nil {
			return err
		}
		if err := s.Send(ctx, reply); err != nil {
			return err
		}
	}
}

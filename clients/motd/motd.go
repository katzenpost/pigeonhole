// Package motd fetches the message of the day from the service in services/motd.
package motd

import (
	"context"
	"errors"

	ps "github.com/katzenpost/katzenpost/pigeonservice"

	"github.com/katzenpost/pigeonhole_protocols/services/motd"
)

// ErrBadReply reports a reply that does not decode.
var ErrBadReply = errors.New("motd: bad reply")

// Fetch asks for the message of the day over a connection to the service.
func Fetch(ctx context.Context, conn *ps.Conn) (*motd.Reply, error) {
	req, err := ps.Marshal(&motd.Request{})
	if err != nil {
		return nil, err
	}
	if err := conn.Send(ctx, req); err != nil {
		return nil, err
	}
	raw, err := conn.Recv(ctx)
	if err != nil {
		return nil, err
	}
	var reply motd.Reply
	if err := ps.Unmarshal(raw, &reply); err != nil {
		return nil, ErrBadReply
	}
	return &reply, nil
}

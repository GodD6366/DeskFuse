package clipboard

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"io"
	"testing"
	"time"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/bridge"
)

func TestBridgeSourceUsesRealFrameProtocol(t *testing.T) {
	requestsR, requestsW := io.Pipe()
	repliesR, repliesW := io.Pipe()
	defer requestsR.Close()
	defer requestsW.Close()
	defer repliesR.Close()
	defer repliesW.Close()

	client := bridge.New(repliesR, requestsW)
	source := NewBridgeSource(client)

	type frame struct {
		Op     string
		Params Query
	}
	frames := make(chan frame, 1)
	go func() {
		var header [4]byte
		if _, err := io.ReadFull(requestsR, header[:]); err != nil {
			return
		}
		body := make([]byte, binary.BigEndian.Uint32(header[:]))
		if _, err := io.ReadFull(requestsR, body); err != nil {
			return
		}
		var request struct {
			ID     string `json:"id"`
			Op     string `json:"op"`
			Params Query  `json:"params"`
		}
		if json.Unmarshal(body, &request) != nil {
			return
		}
		frames <- frame{Op: request.Op, Params: request.Params}

		result, err := json.Marshal(Page{
			Items:      []Item{{ID: 42, Preview: "real preview"}},
			NextCursor: &Cursor{SortAtMS: 500, ID: 42},
		})
		if err != nil {
			return
		}
		response, err := json.Marshal(map[string]any{
			"v":      1,
			"id":     request.ID,
			"ok":     true,
			"result": json.RawMessage(result),
		})
		if err != nil {
			return
		}
		binary.BigEndian.PutUint32(header[:], uint32(len(response)))
		_, _ = repliesW.Write(header[:])
		_, _ = repliesW.Write(response)
	}()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	page, err := source.Query(ctx, Query{Op: "items", Search: "hello", Limit: PageSize})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].ID != 42 || page.Items[0].Preview != "real preview" {
		t.Fatalf("unexpected page: %+v", page)
	}
	if page.NextCursor == nil || page.NextCursor.ID != 42 {
		t.Fatalf("next cursor = %+v, want id 42", page.NextCursor)
	}

	select {
	case request := <-frames:
		if request.Op != BridgeQueryOperation {
			t.Fatalf("bridge op = %q, want %q", request.Op, BridgeQueryOperation)
		}
		if request.Params.Op != "items" || request.Params.Search != "hello" || request.Params.Limit != PageSize {
			t.Fatalf("unexpected query params: %+v", request.Params)
		}
		if request.Params.IncludeTotal {
			t.Fatal("IncludeTotal = true, want false to avoid a full count")
		}
	case <-time.After(time.Second):
		t.Fatal("bridge host received no request")
	}
}

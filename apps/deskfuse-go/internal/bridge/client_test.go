package bridge

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"io"
	"testing"
	"time"
)

func TestCallRealFrames(t *testing.T) {
	requestsR, requestsW := io.Pipe()
	repliesR, repliesW := io.Pipe()
	defer requestsR.Close()
	defer requestsW.Close()
	defer repliesR.Close()
	defer repliesW.Close()
	client := New(repliesR, requestsW)
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
			ID string `json:"id"`
		}
		if json.Unmarshal(body, &request) != nil {
			return
		}
		response, _ := json.Marshal(map[string]any{"v": 1, "id": request.ID, "ok": true, "result": map[string]any{"version": 1}})
		binary.BigEndian.PutUint32(header[:], uint32(len(response)))
		repliesW.Write(header[:])
		repliesW.Write(response)
	}()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	var result struct {
		Version int `json:"version"`
	}
	if err := client.Call(ctx, "hello", map[string]any{}, &result); err != nil {
		t.Fatal(err)
	}
	if result.Version != 1 {
		t.Fatalf("unexpected response: %+v", result)
	}
}

func TestRejectOversizedReply(t *testing.T) {
	reader, writer := io.Pipe()
	defer reader.Close()
	defer writer.Close()
	client := New(reader, io.Discard)
	go func() { var h [4]byte; binary.BigEndian.PutUint32(h[:], maxReplyBytes+1); writer.Write(h[:]) }()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	err := client.Call(ctx, "hello", nil, nil)
	if err == nil || err == context.DeadlineExceeded {
		t.Fatalf("expected invalid frame error, got %v", err)
	}
}

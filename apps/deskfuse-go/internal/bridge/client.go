// Package bridge implements the private native-host control channel.
package bridge

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sync"
	"sync/atomic"
)

const maxReplyBytes = 32 << 20
const maxRequestBytes = 256 << 10

type reply struct {
	Version int             `json:"v"`
	ID      string          `json:"id"`
	OK      bool            `json:"ok"`
	Result  json.RawMessage `json:"result"`
	Error   *RemoteError    `json:"error"`
	Event   string          `json:"event"`
}

type RemoteError struct {
	Code      string `json:"code"`
	Message   string `json:"message"`
	Retryable bool   `json:"retryable"`
}

func (e *RemoteError) Error() string { return e.Message }

type outcome struct {
	reply reply
	err   error
}

// Client allows concurrent calls while the host replies out of order.
// The caller owns the pipes and closes them when the application exits.
type Client struct {
	writer  io.Writer
	writeMu sync.Mutex
	mu      sync.Mutex
	pending map[string]chan outcome
	stopped error
	next    atomic.Uint64
	events  chan Event
}

type Event struct {
	Name   string
	Params json.RawMessage
}

func (c *Client) Events() <-chan Event { return c.events }

func New(reader io.Reader, writer io.Writer) *Client {
	c := &Client{writer: writer, pending: make(map[string]chan outcome), events: make(chan Event, 64)}
	go c.read(reader)
	return c
}

func (c *Client) Call(ctx context.Context, op string, params any, result any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	id := fmt.Sprint(c.next.Add(1))
	data, err := json.Marshal(struct {
		Version int    `json:"v"`
		ID      string `json:"id"`
		Op      string `json:"op"`
		Params  any    `json:"params"`
	}{1, id, op, params})
	if err != nil {
		return err
	}
	if len(data) > maxRequestBytes {
		return errors.New("请求超过桥接大小限制")
	}
	ch := make(chan outcome, 1)
	c.mu.Lock()
	if c.stopped != nil {
		err = c.stopped
		c.mu.Unlock()
		return err
	}
	c.pending[id] = ch
	c.mu.Unlock()
	defer func() { c.mu.Lock(); delete(c.pending, id); c.mu.Unlock() }()
	frame := make([]byte, 4+len(data))
	binary.BigEndian.PutUint32(frame, uint32(len(data)))
	copy(frame[4:], data)
	c.writeMu.Lock()
	for len(frame) > 0 {
		var n int
		n, err = c.writer.Write(frame)
		if err != nil {
			break
		}
		if n <= 0 {
			err = io.ErrShortWrite
			break
		}
		frame = frame[n:]
	}
	c.writeMu.Unlock()
	if err != nil {
		c.fail(err)
		return err
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case value := <-ch:
		if value.err != nil {
			return value.err
		}
		if !value.reply.OK {
			if value.reply.Error != nil {
				return value.reply.Error
			}
			return errors.New("桥接返回失败但缺少错误详情")
		}
		if result == nil {
			return nil
		}
		return json.Unmarshal(value.reply.Result, result)
	}
}

func (c *Client) fail(err error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.stopped != nil {
		return
	}
	c.stopped = err
	for id, ch := range c.pending {
		ch <- outcome{err: err}
		delete(c.pending, id)
	}
}

func (c *Client) read(reader io.Reader) {
	defer close(c.events)
	for {
		var header [4]byte
		if _, err := io.ReadFull(reader, header[:]); err != nil {
			c.fail(err)
			return
		}
		length := binary.BigEndian.Uint32(header[:])
		if length == 0 || length > maxReplyBytes {
			c.fail(errors.New("桥接响应帧大小无效"))
			return
		}
		data := make([]byte, int(length))
		if _, err := io.ReadFull(reader, data); err != nil {
			c.fail(err)
			return
		}
		var value reply
		if err := json.Unmarshal(data, &value); err != nil {
			c.fail(err)
			return
		}
		if value.Version != 1 {
			c.fail(errors.New("桥接协议版本不兼容"))
			return
		}
		if value.Event != "" {
			select {
			case c.events <- Event{Name: value.Event, Params: value.Result}:
			default:
				c.fail(errors.New("原生窗口事件队列已满"))
				return
			}
			continue
		}
		if value.ID == "" {
			c.fail(errors.New("宿主终止了桥接通道"))
			return
		}
		c.mu.Lock()
		ch := c.pending[value.ID]
		delete(c.pending, value.ID)
		c.mu.Unlock()
		if ch != nil {
			ch <- outcome{reply: value}
		}
	}
}

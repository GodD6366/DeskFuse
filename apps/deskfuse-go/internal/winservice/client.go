// Package winservice connects the shared Go UI to the existing Windows desktop
// service over its authenticated named pipe.
//
// The service protocol is defined in docs/WINDOWS_DESKTOP_IPC.md and
// apps/deskfuse-service/src/desktop_protocol.rs: one request and one response
// per pipe connection, a four-byte little-endian JSON length prefix, protocol
// version 2, and a single 0x06 byte acknowledging a fully read response.
//
// This package owns only the transport and the translation into the shared UI
// snapshot. Identity, trust, networking and clipboard policy stay in the
// service; the UI never opens a second runtime.
package winservice

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sync/atomic"
)

// MaxFrame bounds every request and response frame the service accepts.
const MaxFrame = 24 * 1024 * 1024

// ProtocolVersion is the desktop IPC version this client speaks.
const ProtocolVersion = 2

// Dialer opens one authenticated pipe connection. A request and its response
// share that connection; the client closes it after acknowledging the response.
type Dialer func(ctx context.Context) (io.ReadWriteCloser, error)

// RemoteError is a typed rejection reported by the service.
type RemoteError struct {
	Code      string
	Message   string
	Retryable bool
}

func (e *RemoteError) Error() string {
	if e.Code == "" {
		return e.Message
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Client sends commands and decodes their results.
type Client struct {
	dial Dialer
	next atomic.Uint64
}

// New returns a client that opens a connection per command.
func New(dial Dialer) *Client { return &Client{dial: dial} }

type requestFrame struct {
	Version   int    `json:"version"`
	RequestID uint64 `json:"request_id"`
	Command   any    `json:"command"`
}

type responseFrame struct {
	Version   int              `json:"version"`
	RequestID uint64           `json:"request_id"`
	Result    *json.RawMessage `json:"result"`
}

// Call sends one command and returns the service's result object.
//
// The result must be a single-key map naming the reply, as the protocol
// serializes it: {"Snapshot": …}, {"Poll": …}, {"Queued": …},
// {"Rejected": …} or "UnsupportedVersion". Rejections become *RemoteError.
func (c *Client) Call(ctx context.Context, command any) (map[string]json.RawMessage, error) {
	if c.dial == nil {
		return nil, errors.New("Windows 服务未连接：缺少命名管道连接")
	}
	requestID := c.next.Add(1)
	payload, err := json.Marshal(requestFrame{Version: ProtocolVersion, RequestID: requestID, Command: command})
	if err != nil {
		return nil, err
	}
	if len(payload) > MaxFrame {
		return nil, errors.New("请求超过服务帧大小限制")
	}

	conn, err := c.dial(ctx)
	if err != nil {
		return nil, fmt.Errorf("无法连接 DeskFuse 服务: %w", err)
	}
	defer conn.Close()
	// Pipe reads cannot be interrupted directly; closing the connection is the
	// only portable way to abandon a stalled exchange.
	stop := context.AfterFunc(ctx, func() { _ = conn.Close() })
	defer stop()

	if err := writeFrame(conn, payload); err != nil {
		return nil, fmt.Errorf("写入服务请求失败: %w", err)
	}
	reply, err := readFrame(conn)
	if err != nil {
		return nil, fmt.Errorf("读取服务响应失败: %w", err)
	}
	// Acknowledge the fully read response before closing. The service keeps the
	// pipe instance until this byte or the client closes, so a dropped write is
	// not an error the caller can act on.
	_, _ = conn.Write([]byte{0x06})

	var frame responseFrame
	if err := json.Unmarshal(reply, &frame); err != nil {
		return nil, fmt.Errorf("服务响应不是有效 JSON: %w", err)
	}
	if frame.Version != ProtocolVersion {
		return nil, fmt.Errorf("服务协议版本不兼容: %d", frame.Version)
	}
	if frame.RequestID != requestID {
		return nil, fmt.Errorf("服务响应编号不匹配: 期望 %d，收到 %d", requestID, frame.RequestID)
	}
	if frame.Result == nil {
		return nil, errors.New("服务响应缺少结果")
	}
	var envelope map[string]json.RawMessage
	if err := json.Unmarshal(*frame.Result, &envelope); err != nil {
		return nil, fmt.Errorf("服务结果不是有效对象: %w", err)
	}
	if rejected, ok := envelope["Rejected"]; ok {
		return nil, rejectionError(rejected)
	}
	if _, ok := envelope["UnsupportedVersion"]; ok {
		return nil, &RemoteError{Code: "unsupported_version", Message: "DeskFuse 服务版本不兼容，请更新后再试。"}
	}
	return envelope, nil
}

func rejectionError(raw json.RawMessage) error {
	var rejected struct {
		Code          string `json:"code"`
		Message       string `json:"message"`
		BoundaryError string `json:"boundary_error"`
	}
	if err := json.Unmarshal(raw, &rejected); err != nil {
		return &RemoteError{Code: "rejected", Message: "服务拒绝了请求"}
	}
	code := rejected.BoundaryError
	if code == "" {
		code = rejected.Code
	}
	message := rejected.Message
	if message == "" {
		message = "服务拒绝了请求"
	}
	return &RemoteError{Code: code, Message: message, Retryable: code == "session_unavailable" || code == "service_busy"}
}

func writeFrame(w io.Writer, payload []byte) error {
	var header [4]byte
	binary.LittleEndian.PutUint32(header[:], uint32(len(payload)))
	if _, err := w.Write(header[:]); err != nil {
		return err
	}
	_, err := w.Write(payload)
	return err
}

func readFrame(r io.Reader) ([]byte, error) {
	var header [4]byte
	if _, err := io.ReadFull(r, header[:]); err != nil {
		return nil, err
	}
	length := binary.LittleEndian.Uint32(header[:])
	if length == 0 || int(length) > MaxFrame {
		return nil, errors.New("服务响应帧大小无效")
	}
	data := make([]byte, length)
	if _, err := io.ReadFull(r, data); err != nil {
		return nil, err
	}
	return data, nil
}

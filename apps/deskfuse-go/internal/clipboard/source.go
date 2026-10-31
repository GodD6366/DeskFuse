package clipboard

import (
	"context"
	"errors"

	"github.com/GodD6366/DeskFuse/apps/deskfuse-go/internal/bridge"
)

// BridgeQueryOperation is the private native-host operation for clipboard
// library reads. The params are the engine's complete `items` request; the
// result is the engine's ClipboardLibraryPage JSON.
//
// The native host must implement this operation when it starts the Go UI with
// DESKFUSE_MYGO_BRIDGE=stdio-v1. Without it the page stays unavailable; the Go
// side never falls back to sample data.
const BridgeQueryOperation = "library.query"

// Source reads one page of clipboard history.
type Source interface {
	Query(ctx context.Context, request Query) (Page, error)
}

// BridgeSource reads clipboard history through the stdio bridge client.
type BridgeSource struct {
	client *bridge.Client
}

// NewBridgeSource wraps a bridge client. A nil client yields a source that
// reports an initialization error instead of panicking.
func NewBridgeSource(client *bridge.Client) *BridgeSource {
	return &BridgeSource{client: client}
}

func (s *BridgeSource) Query(ctx context.Context, request Query) (Page, error) {
	if s == nil || s.client == nil {
		return Page{}, errors.New("剪贴板桥接客户端未初始化")
	}
	var page Page
	if err := s.client.Call(ctx, BridgeQueryOperation, request, &page); err != nil {
		return Page{}, err
	}
	return page, nil
}

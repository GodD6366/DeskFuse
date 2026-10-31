package main

import (
	"encoding/json"
	"testing"
)

func TestActiveTransfersDropsCompleted(t *testing.T) {
	a := &auxiliaryWindows{dismissed: map[string]bool{}}
	a.last = auxiliaryState{Transfers: []auxiliaryTransfer{
		{ID: "1", Phase: "sending"},
		{ID: "2", Phase: "completed"},
		{ID: "3", Phase: "failed"},
	}}
	active := a.activeTransfers()
	if len(active) != 2 || active[0].ID != "1" || active[1].ID != "3" {
		t.Fatalf("active = %+v", active)
	}
}

func TestDismissedTransferStaysHiddenUntilItDisappears(t *testing.T) {
	a := &auxiliaryWindows{dismissed: map[string]bool{}}
	a.last = auxiliaryState{Transfers: []auxiliaryTransfer{{ID: "7", Phase: "failed"}}}
	a.dismissed["7"] = true
	if active := a.activeTransfers(); len(active) != 0 {
		t.Fatalf("dismissed transfer still active: %+v", active)
	}
	// 快照里再次出现同一条失败记录时仍然保持关闭。
	if active := a.activeTransfers(); len(active) != 0 {
		t.Fatalf("dismissal was not stable: %+v", active)
	}
	// 传输从快照消失后释放记录，避免无限增长。
	a.last = auxiliaryState{}
	a.activeTransfers()
	if a.dismissed["7"] {
		t.Fatal("dismissal for a vanished transfer was retained")
	}
}

func TestDecodeAuxiliaryState(t *testing.T) {
	raw := json.RawMessage(`{"pairing":["a"],"transfers":[{"id":"9","phase":"delivering"}]}`)
	state := decodeAuxiliaryState(raw)
	if len(state.Pairing) != 1 || state.Pairing[0] != "a" {
		t.Fatalf("pairing = %+v", state.Pairing)
	}
	if len(state.Transfers) != 1 || state.Transfers[0].Phase != "delivering" {
		t.Fatalf("transfers = %+v", state.Transfers)
	}
	if empty := decodeAuxiliaryState(nil); len(empty.Pairing) != 0 || len(empty.Transfers) != 0 {
		t.Fatalf("empty state = %+v", empty)
	}
}

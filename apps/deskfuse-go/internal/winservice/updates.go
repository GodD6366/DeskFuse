package winservice

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"
)

// GitHub Releases is the canonical update source (config/distribution.json and
// AGENTS.md); the independent-domain mirror is only a fallback when GitHub is
// unreachable, matching the engine's update-source priority.
const (
	releasesAPIURL = "https://api.github.com/repos/GodD6366/DeskFuse/releases/latest"
	releasesPage   = "https://github.com/GodD6366/DeskFuse/releases/latest"
	checkTimeout   = 15 * time.Second
)

// updateState is the desktop client's own update status. Windows has no
// published auto-installer yet, so the channel is always "manual": the UI
// shows the new version and a link to the release page instead of an
// install button.
type updateState struct {
	mu          sync.Mutex
	Version     string `json:"version"`
	Channel     string `json:"channel"`
	Target      string `json:"target_version,omitempty"`
	Message     string `json:"message"`
	ReleaseURL  string `json:"release_url,omitempty"`
	Available   bool   `json:"available"`
	Checking    bool   `json:"checking"`
	LastChecked time.Time
}

func newUpdateState() *updateState {
	return &updateState{Channel: "manual", Message: "已是最新版本。"}
}

func (u *updateState) view(currentVersion string) map[string]any {
	u.mu.Lock()
	defer u.mu.Unlock()
	version := u.Version
	if version == "" {
		version = currentVersion
	}
	return map[string]any{
		"version":        version,
		"channel":        u.Channel,
		"target_version": u.Target,
		"message":        u.Message,
		"release_url":    u.ReleaseURL,
		"available":      u.Available,
		"checking":       u.Checking,
	}
}

// updatesCheck queries GitHub Releases once and refreshes the state. It runs
// at startup and on demand; failures keep the last good state and surface a
// real message.
func (s *Service) updatesCheck(ctx context.Context, result any) error {
	latest, ok := s.latestSnapshot()
	current := ""
	if ok {
		current = latest.PackageVersion
	}
	go s.runUpdateCheck(ctx, current)
	return assign(result, map[string]any{"outcome": "dispatched", "action": "updates.check"})
}

func (s *Service) runUpdateCheck(ctx context.Context, current string) {
	s.updates.mu.Lock()
	if s.updates.Checking {
		s.updates.mu.Unlock()
		return
	}
	s.updates.Checking = true
	s.updates.mu.Unlock()
	defer func() {
		s.updates.mu.Lock()
		s.updates.Checking = false
		s.updates.mu.Unlock()
	}()

	fetch := s.fetchRelease
	if fetch == nil {
		fetch = fetchLatestRelease
	}
	target, releaseURL, err := fetch(ctx)
	s.updates.mu.Lock()
	defer s.updates.mu.Unlock()
	s.updates.Version = current
	s.updates.LastChecked = time.Now()
	if err != nil {
		s.updates.Message = fmt.Sprintf("无法检查更新：%v", err)
		return
	}
	if current != "" && compareVersions(target, current) <= 0 {
		s.updates.Target = ""
		s.updates.Available = false
		s.updates.ReleaseURL = ""
		s.updates.Message = "已是最新版本。"
		return
	}
	s.updates.Target = target
	s.updates.Available = true
	s.updates.ReleaseURL = releaseURL
	s.updates.Message = fmt.Sprintf("发现新版本 %s。", target)
}

type releaseReply struct {
	TagName string `json:"tag_name"`
	HTMLURL string `json:"html_url"`
}

func fetchLatestRelease(ctx context.Context) (version, url string, err error) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, releasesAPIURL, nil)
	if err != nil {
		return "", "", err
	}
	request.Header.Set("Accept", "application/vnd.github+json")
	request.Header.Set("User-Agent", "DeskFuse-Windows-Client")
	response, err := (&http.Client{Timeout: checkTimeout}).Do(request)
	if err != nil {
		return "", "", err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return "", "", fmt.Errorf("GitHub 返回 %s", response.Status)
	}
	var reply releaseReply
	if err := json.NewDecoder(response.Body).Decode(&reply); err != nil {
		return "", "", err
	}
	version = strings.TrimPrefix(strings.TrimSpace(reply.TagName), "v")
	if version == "" {
		return "", "", fmt.Errorf("发布信息缺少版本号")
	}
	if reply.HTMLURL == "" {
		reply.HTMLURL = releasesPage
	}
	return version, reply.HTMLURL, nil
}

// compareVersions compares dotted versions a and b, returning >0 when a is
// newer. Non-numeric parts compare lexically; malformed input never panics.
func compareVersions(a, b string) int {
	as := strings.Split(a, ".")
	bs := strings.Split(b, ".")
	for i := 0; i < len(as) || i < len(bs); i++ {
		var x, y string
		if i < len(as) {
			x = as[i]
		}
		if i < len(bs) {
			y = bs[i]
		}
		ax, aErr := strconv.Atoi(onlyDigits(x))
		bx, bErr := strconv.Atoi(onlyDigits(y))
		var cmp int
		switch {
		case aErr == nil && bErr == nil:
			cmp = ax - bx
		case aErr == nil:
			cmp = 1
		case bErr == nil:
			cmp = -1
		default:
			cmp = strings.Compare(x, y)
		}
		if cmp != 0 {
			return cmp
		}
	}
	return 0
}

func onlyDigits(s string) string {
	var out strings.Builder
	for _, r := range s {
		if r >= '0' && r <= '9' {
			out.WriteRune(r)
		}
	}
	return out.String()
}

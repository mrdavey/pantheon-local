package service

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

func (i Info) Link() string {
	return i.URL + "/#" + i.Token
}

func Running(ctx context.Context, dir string) (Info, error) {
	var info Info
	raw, err := os.ReadFile(filepath.Join(dir, "service.json"))
	if err != nil {
		return info, err
	}
	if err := json.Unmarshal(raw, &info); err != nil {
		return info, err
	}
	host, err := url.Parse(info.URL)
	if err != nil || host.Scheme != "http" || host.User != nil || host.Path != "" || host.RawQuery != "" || host.Fragment != "" {
		return Info{}, errors.New("invalid local service address")
	}
	port, portErr := strconv.Atoi(host.Port())
	ip := net.ParseIP(host.Hostname())
	token, tokenErr := hex.DecodeString(info.Token)
	if ip == nil || ip.To4() == nil || !ip.IsLoopback() || portErr != nil || port < 1 || port > 65535 || tokenErr != nil || len(token) != 32 {
		return Info{}, errors.New("invalid local service session")
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, info.URL+"/api/session", nil)
	if err != nil {
		return Info{}, err
	}
	req.Header.Set("Authorization", "Bearer "+info.Token)
	client := http.Client{
		Timeout:       500 * time.Millisecond,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	response, err := client.Do(req)
	if err != nil {
		return Info{}, err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusNoContent {
		return Info{}, errors.New("local service session is unavailable")
	}
	return info, nil
}

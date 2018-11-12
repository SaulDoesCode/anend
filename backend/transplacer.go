package backend

import (
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"errors"
	"fmt"
	"io"
	"io/ioutil"
	"mime"
	"net/http"
	"net/textproto"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"

	"github.com/cornelk/hashmap"
	"github.com/fsnotify/fsnotify"
	"golang.org/x/net/html"
)

var (
	// Compressable - list of compressable file types, append to it if needed
	Compressable = []string{"", ".txt", ".htm", ".html", ".css", ".toml", ".php", ".js", ".json", ".md", ".mdown", ".xml", ".svg", ".go", ".cgi", ".py", ".pl", ".aspx", ".asp"}
)

// HashMap is an alias of cornelk/hashmap
type HashMap = hashmap.HashMap

// AssetCache is a store for assets
type AssetCache struct {
	Dir   string
	Cache *HashMap

	Expire   time.Duration
	Interval time.Duration

	CacheControl string

	Ticker *time.Ticker

	DevMode bool

	Watch   bool
	Watcher *fsnotify.Watcher
}

// MakeAssetCache prepares a new *AssetCache for use
func MakeAssetCache(dir string, expire time.Duration, interval time.Duration, watch bool) (*AssetCache, error) {
	dir, err := filepath.Abs(dir)
	if err != nil {
		return nil, err
	}
	a := &AssetCache{
		Dir:          dir,
		Cache:        &HashMap{},
		Expire:       expire,
		CacheControl: "private, must-revalidate",
		Watch:        watch,
	}

	a.SetExpiryCheckInterval(interval)

	go func() {
		for now := range a.Ticker.C {
			for kv := range a.Cache.Iter() {
				asset := kv.Value.(*Asset)
				if asset.Loaded.Add(a.Expire).After(now) {
					a.Del(kv.Key.(string))
				}
			}
		}
	}()

	if a.Watch {
		a.Watcher, err = fsnotify.NewWatcher()
		if err != nil {
			panic(fmt.Errorf(
				"air: failed to build coffer watcher: %v",
				err,
			))
		}
		go func() {
			for {
				select {
				case e := <-a.Watcher.Events:
					if a.DevMode {
						fmt.Printf(
							"\nAssetCache watcher event:\n\tfile: %s \n\t event %s\n",
							e.Name,
							e.Op.String(),
						)
					}

					a.Del(e.Name)
					a.Gen(e.Name)
				case err := <-a.Watcher.Errors:
					fmt.Println("AssetCache watcher error: ", err)
				}
			}
		}()
	}

	return a, err
}

// SetExpiryCheckInterval generates a new ticker with a set interval
func (a *AssetCache) SetExpiryCheckInterval(interval time.Duration) {
	if a.Ticker != nil {
		a.Ticker.Stop()
	}
	a.Interval = interval
	a.Ticker = time.NewTicker(interval)
}

// Close stops and clears the AssetCache
func (a *AssetCache) Close() error {
	a.Cache = &HashMap{}
	if a.Ticker != nil {
		a.Ticker.Stop()
	}
	return nil
}

// Gen generates a new Asset
func (a *AssetCache) Gen(name string) (*Asset, error) {
	name = prepPath(a.Dir, name)

	fs, err := os.Stat(name)
	if err != nil {
		return nil, err
	}

	if fs.IsDir() {
		return a.Gen(name + "/index.html")
	}

	f, err := os.Open(name)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	content, err := ioutil.ReadFile(name)
	if err != nil {
		return nil, err
	}

	ext := filepath.Ext(name)

	ContentType := mime.TypeByExtension(ext)

	Compressed := stringsContainsCI(Compressable, ext)

	asset := &Asset{
		ContentType:  ContentType,
		Content:      bytes.NewReader(content),
		Compressed:   Compressed,
		CacheControl: a.CacheControl,
		ModTime:      fs.ModTime(),
		Ext:          ext,
		Name:         fs.Name(),
	}

	if Compressed {
		compressed, err := gzipBytes(content, 9)
		if err != nil {
			return nil, err
		}
		compressedReader := bytes.NewReader(compressed)
		var et []byte
		h := sha256.New()
		_, err = io.Copy(h, compressedReader)
		if err != nil {
			return nil, err
		}
		if et == nil {
			et = h.Sum(nil)
		}
		asset.EtagCompressed = fmt.Sprintf(`"%x"`, et)
		asset.ContentCompressed = compressedReader
	}

	var et []byte
	h := sha256.New()
	_, err = io.Copy(h, f)
	if err != nil {
		return nil, err
	}
	if et == nil {
		et = h.Sum(nil)
	}
	asset.Etag = fmt.Sprintf(`"%x"`, et)

	if err == nil {
		asset.Loaded = time.Now()
		if ext == ".html" {
			list, err := queryPushables(string(content))
			if err == nil {
				asset.PushList = list
			}
		}

		a.Cache.Set(name, asset)
		if a.Watch {
			a.Watcher.Add(name)
		}
	}

	return asset, err
}

// Get fetches an asset
func (a *AssetCache) Get(name string) (*Asset, bool) {
	name = prepPath(a.Dir, name)

	raw, ok := a.Cache.GetStringKey(name)
	if !ok {
		asset, err := a.Gen(name)
		if err != nil && a.DevMode {
			fmt.Println("AssetCache.Get err: ", err, "name: ", name)
		}
		return asset, err == nil
	}
	return raw.(*Asset), ok
}

// Del removes an asset, nb. not the file, the file is fine
func (a *AssetCache) Del(name string) {
	name = prepPath(a.Dir, name)
	a.Cache.Del(name)
	if a.Watch {
		a.Watcher.Remove(name)
	}
}

// ErrAssetNotFound is for when an asset cannot be located/created
var ErrAssetNotFound = errors.New(`no such asset to serve`)

// ServeFileDirect takes a key/filename directly and serves it if it exists and returns an ErrAssetNotFound if it doesn't
func (a *AssetCache) ServeFileDirect(res http.ResponseWriter, req *http.Request, file string) error {
	asset, ok := a.Get(file)
	if !ok {
		return ErrAssetNotFound
	}
	asset.ServeHTTP(res, req)
	return nil
}

// ServeFile parses a key/filename and serves it if it exists and returns an ErrAssetNotFound if it doesn't
func (a *AssetCache) ServeFile(res http.ResponseWriter, req *http.Request, file string) error {
	return a.ServeFileDirect(res, req, prepPath(a.Dir, file))
}

// Asset is an http servable resource
type Asset struct {
	Ext string

	Name string

	ContentType string

	Loaded time.Time

	ModTime time.Time

	Content           *bytes.Reader
	ContentCompressed *bytes.Reader

	CacheControl string

	Etag           string
	EtagCompressed string

	Compressed bool

	PushList []string
}

// ServeHTTP an asset through c *Ctx
func (as *Asset) ServeHTTP(res http.ResponseWriter, req *http.Request) {
	res.Header().Set("Content-Type", as.ContentType)

	if req.TLS != nil && res.Header().Get("Strict-Transport-Security") == "" {
		res.Header().Set("Strict-Transport-Security", "max-age=31536000")
	}

	res.Header().Set("Cache-Control", as.CacheControl)
	if len(as.PushList) > 0 {
		pushWithHeaders(res, req, as.PushList)
	}

	if as.Compressed && strings.Contains(req.Header.Get("Accept-Encoding"), "gzip") {
		res.Header().Set("Etag", as.EtagCompressed)
		res.Header().Set("Content-Encoding", "gzip")
		res.Header().Set("Vary", "accept-encoding")
		http.ServeContent(res, req, as.Name, as.ModTime, as.ContentCompressed)
	} else {
		res.Header().Set("Etag", as.Etag)
		http.ServeContent(res, req, as.Name, as.ModTime, as.Content)
	}
}

func gzipBytes(content []byte, level int) ([]byte, error) {
	var b bytes.Buffer

	gz, err := gzip.NewWriterLevel(&b, level)
	if err != nil {
		return nil, err
	}

	if _, err := gz.Write(content); err != nil {
		return nil, err
	}
	if err := gz.Flush(); err != nil {
		return nil, err
	}
	if err := gz.Close(); err != nil {
		return nil, err
	}

	return b.Bytes(), nil
}

func prepPath(host, file string) string {
	file = path.Clean(file)

	if !strings.Contains(file, host) {
		file = filepath.Join(host, file)
	}

	if file[len(file)-1] == '/' {
		return filepath.Join(file, "index.html")
	}
	return file
}

// HTTP2Push initiates an HTTP/2 server push. This constructs a synthetic request
// using the target and headers, serializes that request into a PUSH_PROMISE
// frame, then dispatches that request using the server's request handlec. The
// target must either be an absolute path (like "/path") or an absolute URL
// that contains a valid host and the same scheme as the parent request. If the
// target is a path, it will inherit the scheme and host of the parent request.
// The headers specifies additional promised request headers. The headers
// cannot include HTTP/2 pseudo headers like ":path" and ":scheme", which
// will be added automatically.
func HTTP2Push(W http.ResponseWriter, target string, headers http.Header) error {
	p, ok := W.(http.Pusher)
	if !ok {
		return nil
	}

	var pos *http.PushOptions
	if l := len(headers); l > 0 {
		pos = &http.PushOptions{
			Header: make(http.Header, l),
		}

		pos.Header.Set("cache-control", "private, must-revalidate")

		for name, values := range headers {
			pos.Header[textproto.CanonicalMIMEHeaderKey(name)] = values
		}
	}

	return p.Push(target, pos)
}

func cloneHeader(h http.Header) http.Header {
	h2 := make(http.Header, len(h))
	for k, vv := range h {
		vv2 := make([]string, len(vv))
		copy(vv2, vv)
		h2[k] = vv2
	}
	return h2
}

func pushWithHeaders(W http.ResponseWriter, R *http.Request, list []string) {
	for _, target := range list {
		reqHeaders := cloneHeader(R.Header)
		reqHeaders.Del("etag")
		for name := range reqHeaders {
			if strings.Contains(name, "if") ||
				strings.Contains(name, "modified") {
				reqHeaders.Del(name)
			}
		}
		err := HTTP2Push(W, target, reqHeaders)
		if DevMode && err != nil {
			fmt.Println("http2 push Error: ", err)
		}
	}
}

func queryPushables(h string) ([]string, error) {
	list := []string{}
	tree, err := html.Parse(strings.NewReader(h))
	if err != nil {
		return list, err
	}

	var f func(*html.Node)
	f = func(n *html.Node) {
		if n.Type == html.ElementNode {
			target := ""
			switch n.Data {
			case "link":
				for _, a := range n.Attr {
					if a.Key == "href" {
						target = a.Val
						break
					}
				}
			case "img", "script":
				for _, a := range n.Attr {
					if a.Key == "src" {
						target = a.Val
						break
					}
				}
			}

			if path.IsAbs(target) {
				list = append(list, target)
			}
		}

		for c := n.FirstChild; c != nil; c = c.NextSibling {
			f(c)
		}
	}

	f(tree)
	return list, err
}

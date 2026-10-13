package backend

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	bolt "go.etcd.io/bbolt"
)

// importDoc is a writ as arangoexport writes it. Dates are read leniently and
// arango's own fields (_id, _rev) are ignored.
type importDoc struct {
	Writ
	Created   string   `json:"created"`
	Edits     []string `json:"edits"`
	Published *bool    `json:"published"` // what anend notes calls public
}

var timeLayouts = []string{time.RFC3339Nano, time.RFC3339, "2006-01-02T15:04:05.999999999", "2006-01-02 15:04:05", "2006-01-02"}

func parseTime(s string) (time.Time, error) {
	s = strings.TrimSpace(s)
	for _, layout := range timeLayouts {
		if t, err := time.Parse(layout, s); err == nil {
			return t, nil
		}
	}
	if n, err := strconv.ParseInt(s, 10, 64); err == nil {
		if n > 1e12 {
			return time.UnixMilli(n), nil
		}
		return time.Unix(n, 0), nil
	}
	return time.Time{}, fmt.Errorf("can't read the date %q", s)
}

// readImport reads an arangoexport dump of the writs collection: either a
// json array (--type json) or one document per line (--type jsonl).
func readImport(r io.Reader) ([]importDoc, error) {
	raw, err := io.ReadAll(r)
	if err != nil {
		return nil, err
	}
	raw = bytes.TrimSpace(raw)
	var docs []importDoc
	if len(raw) > 0 && raw[0] == '[' {
		if err := json.Unmarshal(raw, &docs); err != nil {
			return nil, err
		}
		return docs, nil
	}
	sc := bufio.NewScanner(bytes.NewReader(raw))
	sc.Buffer(make([]byte, 1<<20), 64<<20)
	for line := 1; sc.Scan(); line++ {
		l := bytes.TrimSpace(sc.Bytes())
		if len(l) == 0 {
			continue
		}
		var d importDoc
		if err := json.Unmarshal(l, &d); err != nil {
			return nil, fmt.Errorf("line %d: %w", line, err)
		}
		docs = append(docs, d)
	}
	return docs, sc.Err()
}

// ImportWrits loads writs from an arangoexport dump, keeping their keys,
// titles, markdown, tags, dates, edit history and public flag. A writ whose
// key is already in the store is replaced, so importing twice is harmless.
func (s *Store) ImportWrits(r io.Reader) (int, error) {
	docs, err := readImport(r)
	if err != nil {
		return 0, err
	}
	n := 0
	err = s.db.Update(func(tx *bolt.Tx) error {
		for i, d := range docs {
			w := d.Writ
			if d.Created != "" {
				t, err := parseTime(d.Created)
				if err != nil {
					return fmt.Errorf("writ %d (%s): %w", i+1, w.Key, err)
				}
				w.Created = t
			} else {
				w.Created = time.Now()
			}
			w.Edits = nil
			for _, e := range d.Edits {
				t, err := parseTime(e)
				if err != nil {
					return fmt.Errorf("writ %d (%s): %w", i+1, w.Key, err)
				}
				w.Edits = append(w.Edits, t)
			}
			if d.Published != nil {
				w.Public = *d.Published
			}
			w.Tags = cleanTags(w.Tags)
			if w.Key == "" {
				key, err := nextKey(tx, bWrits)
				if err != nil {
					return err
				}
				w.Key = key
			}
			if w.Markdown != "" {
				w.RenderContent()
			}
			if w.Slug == "" {
				w.Slug = Slugify(w.Title)
			}
			w.Slug = uniqueSlug(tx, w.Slug, w.Key)
			if err := putWrit(tx, &w); err != nil {
				return fmt.Errorf("writ %d (%s): %w", i+1, w.Key, err)
			}
			n++
		}
		return nil
	})
	if err != nil {
		return 0, err
	}
	return n, nil
}

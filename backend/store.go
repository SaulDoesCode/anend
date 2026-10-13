package backend

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	bolt "go.etcd.io/bbolt"
)

// ErrNotFound is returned when a writ, user, session or verifier doesn't exist.
var ErrNotFound = errors.New("not found")

// bucket names. The record buckets hold JSON values under their key; the
// index buckets map a derived key to a record key (or hold empty values when
// the derived key already contains the record key).
var (
	bWrits        = []byte("writs")          // writ key -> Writ JSON
	bWritsBySlug  = []byte("writs_by_slug")  // slug -> writ key
	bWritsByTitle = []byte("writs_by_title") // title -> writ key
	bWritsByTag   = []byte("writs_by_tag")   // tag \x00 writ key -> ""
	bWritsByDate  = []byte("writs_by_date")  // created (sortable) \x00 writ key -> ""
	bUsers        = []byte("users")          // user key -> User JSON
	bUsersByEmail = []byte("users_by_email") // lowercased email -> user key
	bUsersByName  = []byte("users_by_name")  // lowercased username -> user key
	bSessions     = []byte("sessions")       // sha256(token) -> Session JSON
	bVerifiers    = []byte("verifiers")      // sha256(token) -> Verifier JSON

	allBuckets = [][]byte{
		bWrits, bWritsBySlug, bWritsByTitle, bWritsByTag, bWritsByDate,
		bUsers, bUsersByEmail, bUsersByName, bSessions, bVerifiers,
	}
)

// Store is the embedded bbolt database that holds writs, users and sessions.
type Store struct {
	db *bolt.DB
}

// Open opens (or creates) the database file at path, making its directory
// and buckets as needed.
func Open(path string) (*Store, error) {
	if dir := filepath.Dir(path); dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return nil, err
		}
	}
	db, err := bolt.Open(path, 0o600, &bolt.Options{Timeout: 2 * time.Second})
	if err != nil {
		return nil, fmt.Errorf("open %s: %w", path, err)
	}
	err = db.Update(func(tx *bolt.Tx) error {
		for _, name := range allBuckets {
			if _, err := tx.CreateBucketIfNotExists(name); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		db.Close()
		return nil, err
	}
	return &Store{db: db}, nil
}

// Close closes the database file.
func (s *Store) Close() error {
	return s.db.Close()
}

// Path is where the database file lives.
func (s *Store) Path() string {
	return s.db.Path()
}

func getJSON(tx *bolt.Tx, bucket []byte, key string, v any) error {
	raw := tx.Bucket(bucket).Get([]byte(key))
	if raw == nil {
		return ErrNotFound
	}
	return json.Unmarshal(raw, v)
}

func putJSON(tx *bolt.Tx, bucket []byte, key string, v any) error {
	raw, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return tx.Bucket(bucket).Put([]byte(key), raw)
}

func getIndex(tx *bolt.Tx, bucket []byte, key string) (string, bool) {
	v := tx.Bucket(bucket).Get([]byte(key))
	return string(v), v != nil
}

// nextKey hands out numeric keys like arangodb's ("1000001"), skipping any
// that are taken (imported writs keep their old keys).
func nextKey(tx *bolt.Tx, bucket []byte) (string, error) {
	b := tx.Bucket(bucket)
	for {
		n, err := b.NextSequence()
		if err != nil {
			return "", err
		}
		key := fmt.Sprintf("%d", 1000000+n)
		if b.Get([]byte(key)) == nil {
			return key, nil
		}
	}
}

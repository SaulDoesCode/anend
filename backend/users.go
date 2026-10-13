package backend

import (
	"crypto/md5"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/mail"
	"regexp"
	"slices"
	"strings"
	"time"

	bolt "go.etcd.io/bbolt"
)

// Role auth roles/perms
type Role = int64

const (
	// UnverifiedUser user with unconfirmed email
	UnverifiedUser Role = iota + 1
	// VerifiedUser a verified user
	VerifiedUser
	// Admin boss man with all the perms
	Admin
)

// User is an account. People log in with a username and email pair.
type User struct {
	Key         string      `json:"_key,omitempty"`
	Email       string      `json:"email"`
	EmailMD5    string      `json:"emailmd5"`
	Username    string      `json:"username"`
	Description string      `json:"description,omitempty"`
	Created     time.Time   `json:"created,omitzero"`
	Logins      []time.Time `json:"logins,omitempty"`
	Roles       []Role      `json:"roles,omitempty"`
	Friends     []string    `json:"friends,omitempty"`
	Exp         int64       `json:"exp,omitempty"`
	Subscriber  bool        `json:"subscriber,omitempty"`
}

var (
	// ErrUsernameTaken the username belongs to someone with another email
	ErrUsernameTaken = errors.New("that username is taken")
	// ErrEmailTaken the email belongs to someone with another username
	ErrEmailTaken = errors.New("that email belongs to another username")
	// ErrBadDetails the username or email is malformed
	ErrBadDetails = errors.New("invalid username or email")
)

var usernameRe = regexp.MustCompile(`^[a-zA-Z0-9._-]{3,50}$`)

func validUsername(username string) bool {
	return usernameRe.MatchString(username)
}

func validEmail(email string) bool {
	a, err := mail.ParseAddress(email)
	return err == nil && a.Name == "" && a.Address == email && strings.Contains(email[strings.LastIndexByte(email, '@'):], ".")
}

// HasRole check that a user has a particular auth role
func (u *User) HasRole(role Role) bool {
	return slices.Contains(u.Roles, role)
}

// Verified check that a user has logged in with a magic link at least once
func (u *User) Verified() bool {
	return u.HasRole(VerifiedUser)
}

// IsAdmin check that a user is an admin
func (u *User) IsAdmin() bool {
	return u.HasRole(Admin)
}

func (u *User) addRole(role Role) {
	if !u.HasRole(role) {
		u.Roles = append(u.Roles, role)
	}
}

func putUser(tx *bolt.Tx, u *User) error {
	var old User
	switch err := getJSON(tx, bUsers, u.Key, &old); err {
	case nil:
		tx.Bucket(bUsersByEmail).Delete([]byte(strings.ToLower(old.Email)))
		tx.Bucket(bUsersByName).Delete([]byte(strings.ToLower(old.Username)))
	case ErrNotFound:
	default:
		return err
	}
	if k, ok := getIndex(tx, bUsersByEmail, strings.ToLower(u.Email)); ok && k != u.Key {
		return ErrEmailTaken
	}
	if k, ok := getIndex(tx, bUsersByName, strings.ToLower(u.Username)); ok && k != u.Key {
		return ErrUsernameTaken
	}
	if err := tx.Bucket(bUsersByEmail).Put([]byte(strings.ToLower(u.Email)), []byte(u.Key)); err != nil {
		return err
	}
	if err := tx.Bucket(bUsersByName).Put([]byte(strings.ToLower(u.Username)), []byte(u.Key)); err != nil {
		return err
	}
	return putJSON(tx, bUsers, u.Key, u)
}

func userByIndex(tx *bolt.Tx, bucket []byte, value string, u *User) error {
	key, ok := getIndex(tx, bucket, strings.ToLower(value))
	if !ok {
		return ErrNotFound
	}
	return getJSON(tx, bUsers, key, u)
}

// CreateUser makes a new user. Usernames and emails are unique (ignoring case).
func (s *Store) CreateUser(email, username string, roles ...Role) (User, error) {
	if !validEmail(email) || !validUsername(username) {
		return User{}, ErrBadDetails
	}
	if len(roles) == 0 {
		roles = []Role{UnverifiedUser}
	}
	sum := md5.Sum([]byte(strings.ToLower(email)))
	u := User{
		Email:    email,
		EmailMD5: hex.EncodeToString(sum[:]),
		Username: username,
		Created:  time.Now(),
		Roles:    roles,
	}
	err := s.db.Update(func(tx *bolt.Tx) error {
		key, err := nextKey(tx, bUsers)
		if err != nil {
			return err
		}
		u.Key = key
		return putUser(tx, &u)
	})
	return u, err
}

// UserByKey gets a user by key.
func (s *Store) UserByKey(key string) (User, error) {
	var u User
	err := s.db.View(func(tx *bolt.Tx) error { return getJSON(tx, bUsers, key, &u) })
	return u, err
}

// UserByEmail gets a user by email (ignoring case).
func (s *Store) UserByEmail(email string) (User, error) {
	var u User
	err := s.db.View(func(tx *bolt.Tx) error { return userByIndex(tx, bUsersByEmail, email, &u) })
	return u, err
}

// UserByUsername gets a user by username (ignoring case).
func (s *Store) UserByUsername(username string) (User, error) {
	var u User
	err := s.db.View(func(tx *bolt.Tx) error { return userByIndex(tx, bUsersByName, username, &u) })
	return u, err
}

// UpdateUser changes a stored user in place.
func (s *Store) UpdateUser(key string, change func(*User) error) (User, error) {
	var u User
	err := s.db.Update(func(tx *bolt.Tx) error {
		if err := getJSON(tx, bUsers, key, &u); err != nil {
			return err
		}
		if err := change(&u); err != nil {
			return err
		}
		u.Key = key
		return putUser(tx, &u)
	})
	return u, err
}

// IsUsernameAvailable checks that the username is valid and unused.
func (s *Store) IsUsernameAvailable(username string) bool {
	if !validUsername(username) {
		return false
	}
	_, err := s.UserByUsername(username)
	return err == ErrNotFound
}

// FindOrCreateUser returns the user with this email and username, making one
// if neither is in use yet.
func (s *Store) FindOrCreateUser(email, username string) (User, error) {
	if !validEmail(email) || !validUsername(username) {
		return User{}, ErrBadDetails
	}
	if u, err := s.UserByEmail(email); err == nil {
		if !strings.EqualFold(u.Username, username) {
			return User{}, ErrEmailTaken
		}
		return u, nil
	}
	return s.CreateUser(email, username)
}

// MakeAdmin finds or creates the user with this email and makes them an
// admin. A new user gets username, or one made from the email.
func (s *Store) MakeAdmin(email, username string) (User, error) {
	if !validEmail(email) {
		return User{}, ErrBadDetails
	}
	u, err := s.UserByEmail(email)
	if err == ErrNotFound {
		if username == "" {
			username = usernameFromEmail(email)
		}
		u, err = s.CreateUser(email, username, VerifiedUser, Admin)
	}
	if err != nil {
		return u, err
	}
	return s.UpdateUser(u.Key, func(u *User) error {
		u.addRole(Admin)
		return nil
	})
}

func usernameFromEmail(email string) string {
	local := email[:strings.IndexByte(email, '@')]
	name := strings.Map(func(r rune) rune {
		if r < 128 && (r == '.' || r == '_' || r == '-' || 'a' <= r && r <= 'z' || 'A' <= r && r <= 'Z' || '0' <= r && r <= '9') {
			return r
		}
		return -1
	}, local)
	for len(name) < 3 {
		name += "_"
	}
	if len(name) > 50 {
		name = name[:50]
	}
	return name
}

// Session is a login: the cookie holds a random token, the store holds its
// hash so a copy of the database can't be used to log in.
type Session struct {
	User    string    `json:"user"`
	Created time.Time `json:"created"`
	Expires time.Time `json:"expires"`
}

// Verifier is a magic link token waiting to be used.
type Verifier struct {
	User    string    `json:"user"`
	Expires time.Time `json:"expires"`
}

func newToken() string {
	b := make([]byte, 32)
	rand.Read(b)
	return base64.RawURLEncoding.EncodeToString(b)
}

func tokenID(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

// NewSession logs a user in for ttl and returns the session token.
func (s *Store) NewSession(userKey string, ttl time.Duration) (string, error) {
	token := newToken()
	now := time.Now()
	err := s.db.Update(func(tx *bolt.Tx) error {
		var u User
		if err := getJSON(tx, bUsers, userKey, &u); err != nil {
			return err
		}
		u.Logins = append(u.Logins, now)
		if err := putUser(tx, &u); err != nil {
			return err
		}
		return putJSON(tx, bSessions, tokenID(token), Session{User: userKey, Created: now, Expires: now.Add(ttl)})
	})
	return token, err
}

// SessionUser returns the user and session behind a session token. Expired
// sessions are removed and count as not found.
func (s *Store) SessionUser(token string) (User, Session, error) {
	var u User
	var sess Session
	err := s.db.View(func(tx *bolt.Tx) error {
		if err := getJSON(tx, bSessions, tokenID(token), &sess); err != nil {
			return err
		}
		if time.Now().After(sess.Expires) {
			return errExpired
		}
		return getJSON(tx, bUsers, sess.User, &u)
	})
	if err == errExpired {
		s.DeleteSession(token)
		err = ErrNotFound
	}
	return u, sess, err
}

var errExpired = errors.New("expired")

// RenewSession pushes a session's expiry out to ttl from now.
func (s *Store) RenewSession(token string, ttl time.Duration) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		var sess Session
		if err := getJSON(tx, bSessions, tokenID(token), &sess); err != nil {
			return err
		}
		sess.Expires = time.Now().Add(ttl)
		return putJSON(tx, bSessions, tokenID(token), sess)
	})
}

// DeleteSession logs a session out.
func (s *Store) DeleteSession(token string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(bSessions).Delete([]byte(tokenID(token)))
	})
}

// NewVerifier makes a magic link token for a user, good once within ttl.
func (s *Store) NewVerifier(userKey string, ttl time.Duration) (string, error) {
	token := newToken()
	err := s.db.Update(func(tx *bolt.Tx) error {
		if tx.Bucket(bUsers).Get([]byte(userKey)) == nil {
			return ErrNotFound
		}
		return putJSON(tx, bVerifiers, tokenID(token), Verifier{User: userKey, Expires: time.Now().Add(ttl)})
	})
	return token, err
}

// UseVerifier spends a magic link token: it's removed, and the user it
// belongs to is marked verified and returned.
func (s *Store) UseVerifier(token string) (User, error) {
	var u User
	err := s.db.Update(func(tx *bolt.Tx) error {
		var v Verifier
		id := []byte(tokenID(token))
		if err := getJSON(tx, bVerifiers, string(id), &v); err != nil {
			return err
		}
		if err := tx.Bucket(bVerifiers).Delete(id); err != nil {
			return err
		}
		if time.Now().After(v.Expires) {
			return errExpired
		}
		if err := getJSON(tx, bUsers, v.User, &u); err != nil {
			return err
		}
		if !u.Verified() {
			u.Roles = slices.DeleteFunc(u.Roles, func(r Role) bool { return r == UnverifiedUser })
			u.addRole(VerifiedUser)
			return putUser(tx, &u)
		}
		return nil
	})
	if err == errExpired {
		err = ErrNotFound
	}
	return u, err
}

// Prune removes expired sessions and verifiers.
func (s *Store) Prune() error {
	now := time.Now()
	return s.db.Update(func(tx *bolt.Tx) error {
		for _, name := range [][]byte{bSessions, bVerifiers} {
			c := tx.Bucket(name).Cursor()
			for k, v := c.First(); k != nil; k, v = c.Next() {
				var e struct {
					Expires time.Time `json:"expires"`
				}
				if json.Unmarshal(v, &e) == nil && now.After(e.Expires) {
					if err := c.Delete(); err != nil {
						return err
					}
				}
			}
		}
		return nil
	})
}

// SubscriberEmails lists the emails of users who want to hear about new writs.
func (s *Store) SubscriberEmails() ([]string, error) {
	var out []string
	err := s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(bUsers).ForEach(func(_, v []byte) error {
			var u User
			if err := json.Unmarshal(v, &u); err != nil {
				return err
			}
			if u.Subscriber {
				out = append(out, u.Email)
			}
			return nil
		})
	})
	return out, err
}

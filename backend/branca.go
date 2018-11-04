package backend

import (
	"bytes"
	"crypto/rand"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"time"

	"golang.org/x/crypto/chacha20poly1305"
)

const (
	version byte = 0xBA // Branca magic byte
	base62       = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
)

var (
	// ErrInvalidToken ...
	ErrInvalidToken = errors.New("invalid base62 token")
	// ErrInvalidTokenVersion ...
	ErrInvalidTokenVersion = errors.New("invalid token version")
	// ErrBadKeyLength ...
	ErrBadKeyLength = errors.New("bad key length")
	// ErrExpiredToken ...
	ErrExpiredToken = errors.New("token has expired")
)

// Branca holds a key of exactly 32 bytes. The nonce and timestamp are used for acceptance tests.
type Branca struct {
	Key       string
	nonce     string
	ttl       uint32
	timestamp uint32
}

// SetTTL sets a Time To Live (in seconds) on the token for valid tokens.
func (b *Branca) SetTTL(ttl uint32) {
	b.ttl = ttl
}

// setTimeStamp sets a timestamp for testing.
func (b *Branca) setTimeStamp(timestamp uint32) {
	b.timestamp = timestamp
}

// setNonce sets a nonce for testing.
func (b *Branca) setNonce(nonce string) {
	b.nonce = nonce
}

// NewBranca creates a *Branca struct.
func NewBranca(key string) (b *Branca) {
	return &Branca{Key: key}
}

// EncodeWithTime encodes the data matching the format:
// Version (byte) || Timestamp ([4]byte) || Nonce ([24]byte) || Ciphertext ([]byte) || Tag ([16]byte)
func (b *Branca) EncodeWithTime(data string, timeStamp time.Time) (string, error) {
	var timestamp uint32
	var nonce []byte
	if b.timestamp == 0 {
		b.timestamp = uint32(timeStamp.Unix())
	}
	timestamp = b.timestamp

	if len(b.nonce) == 0 {
		nonce = make([]byte, 24)
		if _, err := rand.Read(nonce); err != nil {
			return "", err
		}
	} else {
		noncebytes, err := hex.DecodeString(b.nonce)
		if err != nil {
			return "", ErrInvalidToken
		}
		nonce = noncebytes
	}

	key := bytes.NewBufferString(b.Key).Bytes()
	payload := bytes.NewBufferString(data).Bytes()

	timeBuffer := make([]byte, 4)
	binary.BigEndian.PutUint32(timeBuffer, timestamp)
	header := append(timeBuffer, nonce...)
	header = append([]byte{version}, header...)

	xchacha, err := chacha20poly1305.NewX(key)
	if err != nil {
		return "", ErrBadKeyLength
	}

	ciphertext := xchacha.Seal(nil, nonce, payload, header)

	token := append(header, ciphertext...)
	base62, err := NewBaseEncoding(base62)
	if err != nil {
		return "", err
	}
	return base62.Encode(token), nil
}

// Encode encodes the data matching the format:
// Version (byte) || Timestamp ([4]byte) || Nonce ([24]byte) || Ciphertext ([]byte) || Tag ([16]byte)
func (b *Branca) Encode(data string) (string, error) {
	return b.EncodeWithTime(data, time.Now())
}

// BrancaToken contains all the decomposed parts of a branca token
type BrancaToken struct {
	Payload    string
	Expiration int64
	Timestamp  int64
}

// ExpiresBefore checks whether the token would expire before a certain time
func (tk *BrancaToken) ExpiresBefore(t time.Time) bool {
	return tk.Expiration < t.Unix()
}

// Decode decode token and return payload string, expiration time.Time, and an error if any
func (b *Branca) Decode(data string) (BrancaToken, error) {
	tk := BrancaToken{}

	if len(data) < 62 {
		return tk, ErrInvalidToken
	}
	base62, err := NewBaseEncoding(base62)
	if err != nil {
		return tk, ErrInvalidToken
	}
	token, err := base62.Decode(data)
	if err != nil {
		return tk, ErrInvalidToken
	}
	header := token[0:29]
	ciphertext := token[29:]
	tokenversion := header[0]
	timestamp := binary.BigEndian.Uint32(header[1:5])
	nonce := header[5:]

	tk.Timestamp = int64(timestamp)

	if tokenversion != version {
		return tk, ErrInvalidTokenVersion
	}

	key := bytes.NewBufferString(b.Key).Bytes()

	xchacha, err := chacha20poly1305.NewX(key)
	if err != nil {
		return tk, ErrBadKeyLength
	}
	payload, err := xchacha.Open(nil, nonce, ciphertext, header)
	if err != nil {
		return tk, err
	}

	tk.Payload = bytes.NewBuffer(payload).String()

	if b.ttl != 0 {
		tk.Expiration = int64(timestamp + b.ttl)
		if tk.Expiration < time.Now().Unix() {
			return tk, ErrExpiredToken
		}
	}

	return tk, err
}

// Encoding is a custom base encoding defined by an alphabet.
// It should bre created using NewEncoding function
type Encoding struct {
	base        int
	alphabet    []rune
	alphabetMap map[rune]int
}

// NewBaseEncoding returns a custom base encoder defined by the alphabet string.
// The alphabet should contain non-repeating characters.
// Ordering is important.
// Example alphabets:
//   - base2: 01
//   - base16: 0123456789abcdef
//   - base32: 0123456789ABCDEFGHJKMNPQRSTVWXYZ
//   - base62: 0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ
func NewBaseEncoding(alphabet string) (*Encoding, error) {
	runes := []rune(alphabet)
	runeMap := make(map[rune]int)

	for i := 0; i < len(runes); i++ {
		if _, ok := runeMap[runes[i]]; ok {
			return nil, errors.New("ambiguous alphabet")
		}

		runeMap[runes[i]] = i
	}

	return &Encoding{
		base:        len(runes),
		alphabet:    runes,
		alphabetMap: runeMap,
	}, nil
}

// Encode function receives a byte slice and encodes it to a string using the alphabet provided
func (e *Encoding) Encode(source []byte) string {
	if len(source) == 0 {
		return ""
	}

	digits := []int{0}

	for i := 0; i < len(source); i++ {
		carry := int(source[i])

		for j := 0; j < len(digits); j++ {
			carry += digits[j] << 8
			digits[j] = carry % e.base
			carry = carry / e.base
		}

		for carry > 0 {
			digits = append(digits, carry%e.base)
			carry = carry / e.base
		}
	}

	var res bytes.Buffer

	for k := 0; source[k] == 0 && k < len(source)-1; k++ {
		res.WriteRune(e.alphabet[0])
	}

	for q := len(digits) - 1; q >= 0; q-- {
		res.WriteRune(e.alphabet[digits[q]])
	}

	return res.String()
}

// Decode function decodes a string previously obtained from Encode, using the same alphabet and returns a byte slice
// In case the input is not valid an arror will be returned
func (e *Encoding) Decode(source string) ([]byte, error) {
	if len(source) == 0 {
		return []byte{}, nil
	}

	runes := []rune(source)

	bytes := []byte{0}
	for i := 0; i < len(source); i++ {
		value, ok := e.alphabetMap[runes[i]]

		if !ok {
			return nil, errors.New("Non Base Character")
		}

		carry := int(value)

		for j := 0; j < len(bytes); j++ {
			carry += int(bytes[j]) * e.base
			bytes[j] = byte(carry & 0xff)
			carry >>= 8
		}

		for carry > 0 {
			bytes = append(bytes, byte(carry&0xff))
			carry >>= 8
		}
	}

	for k := 0; runes[k] == e.alphabet[0] && k < len(runes)-1; k++ {
		bytes = append(bytes, 0)
	}

	// Reverse bytes
	for i, j := 0, len(bytes)-1; i < j; i, j = i+1, j-1 {
		bytes[i], bytes[j] = bytes[j], bytes[i]
	}

	return bytes, nil
}

package backend

import (
	"bytes"
	"compress/gzip"
	"crypto/md5"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/ioutil"
	"os"
	"strconv"

	valid "github.com/asaskevich/govalidator"
	"github.com/microcosm-cc/bluemonday"
	"github.com/tidwall/gjson"
	"gopkg.in/russross/blackfriday.v2"
)

type obj = map[string]interface{}

var (
	RandomDictionary = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
)

func validUsername(username string) bool {
	return valid.Matches(username, `^[a-zA-Z0-9._-]{3,50}$`)
}

func validEmail(email string) bool {
	return valid.IsEmail(email)
}

func validUsernameAndEmail(username string, email string) bool {
	return validEmail(email) && validUsername(username)
}

func check(err error) error {
	if err != nil {
		fmt.Println(err)
	}
	return err
}

func critCheck(err error) {
	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
}

func stringInSlice(str string, list []string) bool {
	for _, v := range list {
		if v == str {
			return true
		}
	}
	return false
}

func removeStringInSlice(s []string, r string) []string {
	for i, v := range s {
		if v == r {
			return append(s[:i], s[i+1:]...)
		}
	}
	return s
}

func CompressBytes(data []byte) ([]byte, error) {
	var buff bytes.Buffer
	gz, err := gzip.NewWriterLevel(&buff, 9)
	if err != nil {
		return nil, err
	}
	_, err = gz.Write(data)
	if err != nil {
		return nil, err
	}
	err = gz.Flush()
	if err != nil {
		return nil, err
	}
	err = gz.Close()
	if err != nil {
		return nil, err
	}
	return buff.Bytes(), err
}

func Uncompress(data []byte) ([]byte, error) {
	reader := bytes.NewReader(data)
	gzipReader, err := gzip.NewReader(reader)
	if err != nil {
		return nil, err
	}
	return ioutil.ReadAll(gzipReader)
}

func randBytes(size int) []byte {

	bits := make([]byte, size)
	rand.Read(bits)
	for k, v := range bits {
		bits[k] = RandomDictionary[v%byte(len(RandomDictionary))]
	}
	return bits
}

func randStr(size int) string {
	return string(randBytes(size))
}

func GetMD5Hash(text string) string {
	hasher := md5.New()
	hasher.Write([]byte(text))
	return hex.EncodeToString(hasher.Sum(nil))
}

func MD5Hash(data []byte) string {
	hasher := md5.New()
	hasher.Write(data)
	return hex.EncodeToString(hasher.Sum(nil))
}

func JSONbody(c ctx) (gjson.Result, error) {
	body, err := ioutil.ReadAll(c.Request().Body)
	if err != nil {
		return gjson.Result{}, err
	}
	return gjson.ParseBytes(body), err
}

func JSONErr(c ctx, code int, err string) error {
	return c.JSONBlob(code, []byte(`{"err":"`+err+`", "valid": false}`))
}

func ReadJSONFile(location string) (gjson.Result, error) {
	var result gjson.Result
	data, err := ioutil.ReadFile(location)
	if err != nil {
		return result, err
	}
	result = gjson.ParseBytes(data)
	return result, nil
}

func UnmarshalJSONFile(location string, marshaled interface{}) error {
	data, err := ioutil.ReadFile(location)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, marshaled)
}

func Int64ToString(n int64) string {
	return strconv.FormatInt(n, 10)
}

// MD2HTML parse markdown and produce sanitized html
func MD2HTML(input []byte) []byte {
	return bluemonday.UGCPolicy().SanitizeBytes(blackfriday.Run(input))
}

func sendError(errorStr string) func(c ctx) error {
	return func(c ctx) error {
		return JSONErr(c, 400, errorStr)
	}
}

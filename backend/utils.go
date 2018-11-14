package backend

import (
	"bytes"
	"crypto/md5"
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io/ioutil"
	"net/http"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"text/template"
	"time"

	"github.com/asaskevich/govalidator"
	"github.com/microcosm-cc/bluemonday"
	"github.com/russross/blackfriday/v2"
	"github.com/vmihailenco/msgpack"
)

var (
	// RandomDictionary the character range of the randomBytes and randomString functions
	RandomDictionary = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
)

func validUsername(username string) bool {
	return govalidator.Matches(username, `^[a-zA-Z0-9._-]{3,50}$`)
}

func validEmail(email string) bool {
	return govalidator.IsEmail(email)
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

// RandBytes generate a random a []byte of a specific size
func RandBytes(size int) []byte {
	bits := make([]byte, size)
	rand.Read(bits)
	for k, v := range bits {
		bits[k] = RandomDictionary[v%byte(len(RandomDictionary))]
	}
	return bits
}

// RandStr generate a random string of a specific size
func RandStr(size int) string {
	return string(RandBytes(size))
}

// GetMD5Hash turn []byte into a MD5 hashed string
func GetMD5Hash(text string) string {
	hasher := md5.New()
	hasher.Write([]byte(text))
	return hex.EncodeToString(hasher.Sum(nil))
}

// MD5Hash turn []byte into a MD5 hashed string
func MD5Hash(data []byte) string {
	hasher := md5.New()
	hasher.Write(data)
	return hex.EncodeToString(hasher.Sum(nil))
}

var (
	bmPolicy = bluemonday.UGCPolicy()
)

func renderMarkdown(input []byte, sanitize bool) []byte {
	if sanitize {
		return bmPolicy.SanitizeBytes(blackfriday.Run(input))
	}
	return blackfriday.Run(input)
}

// SendMsgpack send a msgpack encoded response with a status code
func SendMsgpack(c ctx, code int, msg interface{}) error {
	c.Response().WriteHeader(code)
	c.Response().Header().Set("Content-Type", "application/msgpack")
	return msgpack.NewEncoder(c.Response()).Encode(msg)
}

// SendJSON send a json encoded response with a status code
func SendJSON(c ctx, code int, msg interface{}) error {
	c.Response().WriteHeader(code)
	c.Response().Header().Set("Content-Type", "application/json")
	return json.NewEncoder(c.Response()).Encode(msg)
}

// SendErr send a msgpack encoded error message with this structure {err: "msg"}
func SendErr(c ctx, code int, err string) error {
	return SendMsgpack(c, code, map[string]string{"err": err})
}

// SendErrJSON send a json encoded error message like {"err": "msg"}
func SendErrJSON(c ctx, code int, err string) error {
	return SendJSON(c, code, map[string]string{"err": err})
}

// StaticErrorResponse generates a new error qua CodedResponse
func StaticErrorResponse(code int, msg string) *CodedResponse {
	return MakeCodedResponse(code, msg, obj{"err": msg})
}

// StaticResponse generates a new simple CodedResponse {msg: '...'}
func StaticResponse(code int, msg string) *CodedResponse {
	return MakeCodedResponse(code, msg, obj{"msg": msg})
}

// MakeCodedResponse easilly generate an CodedResponse
func MakeCodedResponse(code int, msg string, primitive interface{}) *CodedResponse {
	cr := &CodedResponse{
		Code: code,
	}
	res, err := msgpack.Marshal(primitive)
	if err != nil {
		panic(err)
	}
	cr.Msgpack = res

	res, err = json.Marshal(primitive)
	if err != nil {
		panic(err)
	}
	cr.JSON = res

	return cr
}

// CodedResponse standardized way to send a prebuilt response with a status code
type CodedResponse struct {
	Code    int
	Msgpack []byte
	JSON    []byte
	Msg     string
}

// Send send off the error through an echo context as msgpack data with a status code
func (e *CodedResponse) Send(c ctx) error {
	return c.Blob(e.Code, "application/msgpack", e.Msgpack)
}

// SendJSON send off the error through an echo context as json data with a status code
func (e *CodedResponse) SendJSON(c ctx) error {
	return c.JSONBlob(e.Code, e.JSON)
}

func (e *CodedResponse) Error() string {
	return e.Msg
}

// UnmarshalJSONFile read json files and go straight to unmarshalling
func UnmarshalJSONFile(location string, marshaled interface{}) error {
	data, err := ioutil.ReadFile(location)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, marshaled)
}

// Int64ToString convert int64 to strings (for ports and stuff when you want to make json less stringy)
func Int64ToString(n int64) string {
	return strconv.FormatInt(n, 10)
}

func execTemplate(temp *template.Template, vars interface{}) ([]byte, error) {
	var buf bytes.Buffer
	err := temp.Execute(&buf, vars)
	return buf.Bytes(), err
}

func unix2time(unix string) (time.Time, error) {
	var tm time.Time
	i, err := strconv.ParseInt(unix, 10, 64)
	if err == nil {
		tm = time.Unix(i, 0)
	}
	return tm, err
}

var (
	pingclient = &http.Client{Transport: &http.Transport{
		TLSClientConfig: &tls.Config{InsecureSkipVerify: true},
	}}
)

// Ping test any http endpoint
func Ping(endpoint string) bool {
	req, err := http.NewRequest(http.MethodGet, endpoint, nil)
	if err != nil {
		if DevMode {
			fmt.Println("had trouble making a new request for pinging ", endpoint, " : ", err)
		}
		return false
	}
	req.Close = true
	res, err := pingclient.Do(req)
	if err != nil {
		if DevMode {
			fmt.Println("had trouble sending a pinging request to ", endpoint, " : ", err)
		}
		return false
	}
	res.Close = true
	res.Body.Close()
	return err == nil && http.StatusOK == res.StatusCode
}

func exeC(cmd string) error {
	fmt.Println(AppName+" trying to run this command -> ", cmd)
	err := exec.Command("/bin/bash", "-c", cmd).Start()
	if err != nil {
		fmt.Printf("command error: %s", err)
	}
	return err
}

func exeCC(cmd string) error {
	fmt.Println(AppName+" trying to run this command -> ", cmd)
	err := exec.Command("/bin/bash", "-c", cmd).Run()
	if err != nil {
		fmt.Printf("command error: %s", err)
	}
	return err
}

func generateDKIM(location string) error {
	pk, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		return err
	}

	asn1bytes, err := x509.MarshalPKIXPublicKey(&pk.PublicKey)
	if err != nil {
		return err
	}

	f, err := os.Create(location + "/" + "dns.txt")
	if err != nil {
		return err
	}

	b64 := base64.StdEncoding.EncodeToString(asn1bytes)
	fmt.Fprintf(f, "v=DKIM1; k=rsa; p=%s", b64)
	f.Close()

	f, err = os.Create(location + "/" + "private.pem")
	if err != nil {
		return err
	}
	err = pem.Encode(f, &pem.Block{
		Type:  "RSA PRIVATE KEY",
		Bytes: x509.MarshalPKCS1PrivateKey(pk),
	})
	if err != nil {
		return err
	}

	fmt.Println("DKIM Generated in :\n\t", location, "\n\ngo now and update your DNS records with what's inside dns.txt\n\t")
	return f.Close()
}

func interfaceSliceToStringSlice(islice []interface{}) []string {
	out := []string{}
	for _, v := range islice {
		out = append(out, v.(string))
	}
	return out
}

// stringsContainsCI reports whether the lists contains a match regardless of its case.
func stringsContainsCI(list []string, match string) bool {
	match = strings.ToLower(match)
	for _, item := range list {
		if strings.ToLower(item) == match {
			return true
		}
	}

	return false
}

package backend

import (
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"math"
	"math/big"
	"net/smtp"
	"os"
	"time"

	"github.com/SaulDoesCode/mailyak"
	"github.com/driusan/dkim"
)

// EmailSettings - email configuration and setup to send authtokens and stuff
var (
	SMTPAuth      smtp.Auth
	DKIMSignature dkim.Signature
	EmailConf     = struct {
		Address  string
		Server   string
		Port     string
		FromName string
		Email    string
		Password string
	}{}
	PrivateDKIMkey *rsa.PrivateKey
)

// startEmailer - initialize the blog's email configuration
func startEmailer() {
	SMTPAuth = smtp.PlainAuth("", EmailConf.Email, EmailConf.Password, EmailConf.Server)

	block, _ := pem.Decode(DKIMKey)
	key, err := x509.ParsePKCS1PrivateKey(block.Bytes)
	if err != nil {
		fmt.Println("the provided dkim key is bad, fix it")
		panic(err)
	}
	PrivateDKIMkey = key

	DKIMSignature, err = dkim.NewSignature(
		"relaxed/relaxed",
		"mail",
		EmailConf.Server,
		[]string{"From", "Date", "Subject", "To"},
	)
	if err != nil {
		fmt.Println("couldn't build a dkim signature")
		panic(err)
	}

	// Send a little test email
	mail := MakeEmail()
	mail.Subject(AppDomain + " server startup notification")
	mail.To(MaintainerEmails...)
	mail.HTML().Set(
		"The " + AppName + " Server is starting up.\n\t" +
			"Everything looks good so far.\n\t" +
			"The startup may have been caused by a crash of some sort, so do check up on that.\n\t" +
			"Other Wise the time of starting is " + time.Now().Format(time.RFC1123) +
			"\n\n\tThat is all.\n\n" +
			"Yours truly\nThe " + AppName + " Server.",
	)
	err = SendEmail(mail)
	if err != nil {
		fmt.Println("emails aren't sending, whats wrong?", err)
		os.Exit(2)
	}

	fmt.Println(`SMTP Emailer Started`)
}

func stopEmailer() {
	//	EmailPool.Close()
}

// MakeEmail builds a new mailyak instance
func MakeEmail() *mailyak.MailYak {
	return mailyak.New(EmailConf.Address, SMTPAuth)
}

// SendEmail send a dkim signed mailyak email
func SendEmail(m *mailyak.MailYak) error {
	m.From(EmailConf.Email)
	m.FromName(EmailConf.FromName)
	mid, err := generateMessageID()
	if err == nil {
		m.AddHeader("Message-Id", mid)
	}
	return m.SignAndSend(DKIMSignature, PrivateDKIMkey)
}

var maxBigInt = big.NewInt(math.MaxInt64)

// generateMessageID generates and returns a string suitable for an RFC 2822
// compliant Message-ID, e.g.:
// <1444789264909237300.3464.1819418242800517193@DESKTOP01>
//
// The following parameters are used to generate a Message-ID:
// - The nanoseconds since Epoch
// - The calling PID
// - A cryptographically random int64
// - The sending hostname
func generateMessageID() (string, error) {
	t := time.Now().UnixNano()
	pid := os.Getpid()
	rint, err := rand.Int(rand.Reader, maxBigInt)
	if err != nil {
		return "", err
	}
	msgid := fmt.Sprintf("<%d.%d.%d@%s>", t, pid, rint, AppDomain)
	return msgid, nil
}

package backend

import (
	"fmt"
	"html/template"
	"path/filepath"
	"time"

	memfile "github.com/SaulDoesCode/echo-memfile"
	"github.com/labstack/echo/middleware"
	"github.com/pelletier/go-toml"
)

var (
	VcodeSize  int
	DevMode    bool
	AppName    string
	Domain     string
	ServerDir  string
	CertDir    string
	TLShost    string
	JWTKey     []byte
	KeyFile    = "./private/key.pem"
	CertFile   = "./private/cert.pem"
	HTTP_Port  = ":80"
	HTTPS_Port = ":443"
	DBAddress  = "127.0.0.1:9080"
	EmailConf  = struct {
		Address  string
		Server   string
		Port     string
		FromTxt  string
		Email    string
		Password string
	}{}
	DB *Database
)

// Init - initialze grimstack.io
func Init(confLocation string) {
	config, err := toml.LoadFile(confLocation)
	critCheck(err)

	serverdir, err := filepath.Abs(config.Get("server.static").(string))
	critCheck(err)
	ServerDir = serverdir

	certdir, err := filepath.Abs(config.Get("server.certdir").(string))
	critCheck(err)
	CertDir = certdir

	DevMode = config.Get("devmode").(bool)

	Domain = config.Get("server.domain").(string)

	JWTKey = []byte(config.Get("auth.jwt").(string))
	VcodeSize = int(config.Get("auth.vcodesize").(int64))

	VerificationEmail = template.Must(template.ParseFiles(config.Get("auth.mailtemplate").(string)))

	EmailConf.Email = config.Get("email.email").(string)
	EmailConf.Server = config.Get("email.server").(string)
	EmailConf.Port = Int64ToString(config.Get("email.port").(int64))
	EmailConf.Password = config.Get("email.password").(string)
	EmailConf.FromTxt = config.Get("email.fromtxt").(string)
	EmailConf.Address = EmailConf.Server + ":" + EmailConf.Port

	DBAddress = config.Get("database.address").(string)

	AppName = config.Get("appname").(string)

	interval := time.Second * 60

	if DevMode {
		KeyFile = config.Get("server.tls.key").(string)
		CertFile = config.Get("server.tls.cert").(string)

		interval = time.Millisecond * 555
		HTTP_Port = ":" + Int64ToString(config.Get("server.dev.http").(int64))
		HTTPS_Port = ":" + Int64ToString(config.Get("server.dev.https").(int64))

		Domain = "localhost" + HTTPS_Port

		Server.Use(middleware.LoggerWithConfig(middleware.LoggerConfig{
			Format: "${method}::${status} ${host}${uri}  \tlag=${latency_human}\n",
		}))
	} else {
		HTTP_Port = ":" + Int64ToString(config.Get("server.http").(int64))
		HTTPS_Port = ":" + Int64ToString(config.Get("server.https").(int64))
	}

	ticker := time.NewTicker(interval)
	go func() {
		for range ticker.C {
			mfi.Update()
		}
	}()

	// serve all static assets from memory
	mfi = memfile.New(Server, ServerDir, DevMode)

	DB = initDB(DBAddress)
	fmt.Println("\nDgraph Connection Established")

	startRoutes()
	startEmailer()
	startUserService()
	startWritService()

	startServer()
	// startServer is a long loop so this will only run after it ends
	DB.Conn.Close()
	ticker.Stop()
}

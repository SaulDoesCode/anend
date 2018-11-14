package backend

import (
	"context"
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"net/http"
	"path/filepath"
	"strings"
	"text/template"
	"time"

	"github.com/CrowdSurge/banner"
	"github.com/SaulDoesCode/echo"
	tr "github.com/SaulDoesCode/transplacer"
	"github.com/integrii/flaggy"
	"github.com/logrusorgru/aurora"
	"github.com/throttled/throttled"
	"github.com/throttled/throttled/store/memstore"
	"github.com/BurntSushi/toml"
	"github.com/json-iterator/go"
	"golang.org/x/crypto/acme/autocert"
)

type ctx = echo.Context
type obj = map[string]interface{}

const oneweek = 7 * 24 * time.Hour

var (
	// AuthEmailHTML html template for authentication emails
	AuthEmailHTML *template.Template
	// AuthEmailTXT html template for authentication emails
	AuthEmailTXT *template.Template
	// PostTemplate html template for post pages
	PostTemplate *template.Template
	// AppName name of this application
	AppName string
	// AppDomain web domain of this application
	AppDomain string
	// DKIMKey private dkim key used for email signing
	DKIMKey []byte
	// DevMode run the app in production or dev-mode
	DevMode = false
	// Tokenator token generator/decoder
	Tokenator *Branca
	// Verinator token generator/decoder for verification codes only
	Verinator *Branca
	// MaintainerEmails the list of people to email if all hell breaks loose
	MaintainerEmails []string
	insecurePort     string
	// AssetsDir path to all the servable static assets
	AssetsDir string
	// AppLocation where this application lives, it's used for self management
	AppLocation string
	// StartupDate when the app started running
	StartupDate time.Time
	// LogQ server logging
	LogQ = []LogEntry{}

	// Server is the echo instance
	Server *echo.Echo

	// Conf contains various information about/for the setup
	Conf *Config

	// Cache serves memory cached (gzipped) static content
	Cache *tr.AssetCache
)

// Init start the backend server
func Init() {
	StartupDate = time.Now()
	dir, err := filepath.Abs(filepath.Dir(os.Args[0]))
	if err != nil {
		log.Fatal(err)
	}
	AppLocation = dir
	fmt.Println("App is located at: ", AppLocation)

	if strings.Contains(AppLocation, "go-build") &&
		(strings.Contains(AppLocation, "Temp") || strings.Contains(AppLocation, "temp")) {
		fmt.Println("warning: self-management will not work with if you ran go run main.go")
	}

	flaggy.Bool(&DevMode, "d", "dev", "launch app in devmode")

	var confloc string
	flaggy.String(&confloc, "c", "conf", "set the configfile location")

	if confloc == "" {
		confloc = "./private/config.toml"
	}

	var donotRatelimit bool
	flaggy.Bool(&donotRatelimit, "nr", "no-ratelimit", "should the server not ratelimit?")

	flaggy.Parse()

	Conf = digestConfig(confloc)

	Conf.DevMode = DevMode

	Server = echo.New()
	Server.Debug = DevMode
	
	if !donotRatelimit && !DevMode {
		store, err := memstore.New(65536)
		if err != nil {
			log.Fatal(err)
		}

		quota := throttled.RateQuota{MaxRate: throttled.PerMin(10), MaxBurst: 4}
		rateLimiter, err := throttled.NewGCRARateLimiter(store, quota)
		if err != nil {
			log.Fatal(err)
		}
	
		httpRateLimiter := throttled.HTTPRateLimiter{
			RateLimiter: rateLimiter,
			VaryBy:      &throttled.VaryBy{Path: true},
		}
		Server.Use(
			echo.WrapMiddleware(func(h http.Handler) http.Handler {
				return httpRateLimiter.RateLimit(h)
			}),
		)
	}
	fmt.Println("\nratelimiting: ", !donotRatelimit)

	if DevMode {
		Conf.Domain = "localhost"
		Conf.AutoCert = Conf.DevAutoCert
		if Conf.DevAddress != "" {
			Conf.Address = Conf.DevAddress
		}
		if Conf.DevSecondaryServerAddress != "" {
			Conf.SecondaryServerAddress = Conf.DevSecondaryServerAddress
		}
	}

	AppName = Conf.AppName
	AppDomain = Conf.Domain

	AssetsDir = Conf.Assets

	if Conf.MaintainerEmail != "" {
		MaintainerEmails = append(MaintainerEmails, Conf.MaintainerEmail)
	}

	fmt.Println("AppName: ", Conf.AppName)
	fmt.Println("Assets: ", AssetsDir)

	mailerObj := Conf.Raw["mailer"].(obj)
	dkimLocation := mailerObj["dkim"].(string)

	fmt.Println("DKIM location is ", dkimLocation, "\n\talways ensure your DNS records are up to date")

	DKIMKey, err = ioutil.ReadFile(dkimLocation + "/private.pem")
	if err != nil {
		fmt.Println("it seems you've no private.pem in your DKIM location\nhang tight, we'll try to generate a new one...\n\t")
		err = generateDKIM(dkimLocation)
		if err != nil {
			fmt.Println("Well, Generating new DKIM credentials has failed, you're on your own with this one")
			os.Exit(2)
		}
		DKIMKey, err = ioutil.ReadFile(dkimLocation + "/private.pem")
		if err != nil {
			fmt.Println("something is still wrong, maybe your DKIM location is invalid, \nor, something is screwy permissions wise.\n you'll have to fix that first")
			os.Exit(2)
		}
	}

	fmt.Println("Firing up: ", AppName+"...")
	fmt.Println("\nDevMode: ", DevMode)

	dbobj := Conf.Raw["db"].(obj)

	addrs := interfaceSliceToStringSlice(dbobj["local_address"].([]interface{}))

	dbname := dbobj["name"].(string)
	dbusername := dbobj["username"].(string)
	dbpassword := dbobj["password"].(string)

	err = setupDB(addrs, dbname, dbusername, dbpassword)
	if err != nil {
		fmt.Println("couldn't connect to DB locally, trying remote connection now...")

		addrs = interfaceSliceToStringSlice(dbobj["address"].([]interface{}))

		err = setupDB(addrs, dbname, dbusername, dbpassword)
		if err != nil {
			fmt.Println(aurora.Brown("couldn't get DB connection going: "), aurora.Red(err))
			panic(err)
		}
	}

	EmailConf.Email = mailerObj["email"].(string)
	EmailConf.Server = mailerObj["server"].(string)
	EmailConf.Port = mailerObj["port"].(string)
	EmailConf.Password = mailerObj["password"].(string)
	EmailConf.FromName = mailerObj["name"].(string)
	EmailConf.Address = EmailConf.Server + ":" + EmailConf.Port

	fmt.Println(EmailConf.Address, EmailConf.Email, EmailConf.FromName)
	startEmailer()

	startDBHealthCheck()
	defer DBHealthTicker.Stop()

	startSelfManaging()

	AuthEmailHTML = template.Must(template.ParseFiles("./templates/authemail.html"))
	AuthEmailTXT = template.Must(template.ParseFiles("./templates/authemail.txt"))
	PostTemplate = template.Must(template.ParseFiles("./templates/post.html"))

	secretsObj := Conf.Raw["secrets"].(obj)
	tokenSecret := secretsObj["token"].(string)
	verifierSecret := secretsObj["verifier"].(string)

	Tokenator = NewBranca(tokenSecret)
	Tokenator.SetTTL(86400 * 7)
	Verinator = NewBranca(verifierSecret)
	Verinator.SetTTL(925)

	initAuth()
	initWrits()

	Server.HTTPErrorHandler = func(err error, c ctx) {
		if err == Err404NotFound {
			Err404NotFound.Send(c)
			return
		}

		cmsg, ok := err.(*CodedResponse)
		if ok {
			cmsg.SendJSON(c)
			return
		}
	
		Server.DefaultHTTPErrorHandler(err, c)
	}

	Server.Pre(
		func (next echo.HandlerFunc) echo.HandlerFunc {
			return func(c ctx) error {
				startTime := time.Now()
				err := next(c)
				endTime := time.Now()

				req := c.Request()
				res := c.Response()
				path := req.RequestURI

				if Cache != nil && req.Method[0] == 'G' && err != nil && !res.Committed {
					_, ok := err.(*CodedResponse)
					if !ok {
						err = Cache.Serve(res.Writer, req)
						if Conf.DevMode && err != nil {
							fmt.Println("Cache.ServeFile error: ", err)
						}
					}
				}

				latency := float64(endTime.Sub(startTime)) / float64(time.Millisecond)
				entry := LogEntry{
					Method:   req.Method,
					Code:     res.Status,
					Latency:  latency,
					Path:     path,
					IP:  	    c.RealIP(),
					Start:    startTime,
					End:      endTime,
					BytesOut: res.Size,
					DevMode:  DevMode,
				}

				if bytesin, err := str2int64(req.Header.Get(echo.HeaderContentLength));
					err == nil && bytesin != 0 {
					entry.BytesIn = bytesin
				}

				if err != nil {
					entry.Err = err.Error()
				}
				
				if (strings.Contains(entry.Err, "code=404") && entry.Code == 200) ||
				strings.Contains(path, ".ico") {
					entry.Err = ""
				}

				if DevMode {
					authpath := strings.Contains(path, "auth")
					if authpath {

						headers := obj{}
						for name, header := range req.Header {
							headers[name] = header[0]
						}
						entry.Headers = headers

						fmt.Println("\n\tAuth Path:\n\n\t")
					}

					if err != nil {
						fmt.Printf(
							"%s:%s %s %gms | client: %s | err: %s\n",
							aurora.Brown(req.Method).String(),
							aurora.Green(res.Status).String(),
							path,
							latency,
							entry.IP,
							aurora.Red(err.Error()),
						)
					} else {
						fmt.Printf(
							"%s:%s %s %gms | client: %s\n",
							aurora.Brown(req.Method).String(),
							aurora.Green(res.Status).String(),
							path,
							latency,
							entry.IP,
						)
					}

					if strings.Contains(path, "auth") {
						fmt.Println("\n\t:Auth Path End\n\t")
					}
				}

				LogQ = append(LogQ, entry)

				return err
			}
		},
	)

	if Conf.Assets != "" {
		assets, err := filepath.Abs(Conf.Assets)
		if err != nil {
			fmt.Println("assets dir error, cannot get absolute path: ", err, assets)
			panic("could not get an absolute path for the assets directory")
		}
		Conf.Assets = assets

		stat, err := os.Stat(Conf.Assets)
		if err != nil {
			fmt.Println("assets dir err: ", err)
			panic("something wrong with the Assets dir/path, best you check what's going on")
		}
		if !stat.IsDir() {
			panic("the path of assets, leads to no folder sir, you best fix that now!")
		}

		cache, err := tr.Make(&tr.AssetCache{
			Dir: Conf.Assets,
			Expire: time.Minute * 45,
			Interval: time.Minute * 2,
			Watch: !Conf.DoNotWatchAssets,
			DevMode: Conf.DevMode,
		})
		if err != nil {
			panic("AssetCache setup failure: " + err.Error())
		}
		Cache = cache
	
		Err404NotFound = MakePageErr(404, "Not Found", "/404.html")

		Cache.NotFoundError = Err404NotFound
		Cache.NotFoundHandler = nil

		defer Cache.Close()
	}

	go func() {
		time.Sleep(2 * time.Second)
		fmt.Printf("\n")
		fmt.Println(aurora.Green("-------------------------"))
		fmt.Println(aurora.Bold(aurora.Magenta(banner.PrintS(AppName))))
		fmt.Println(aurora.Green("-------------------------"))
		fmt.Printf("\n")

		fmt.Println("AutoCert: ", Conf.AutoCert)
		fmt.Println("Server Address: ", Server.TLSServer.Addr)
		fmt.Println("Secondary Server Address: ", Server.Server.Addr)

		fmt.Printf("\n")
	}()

	Server.Server.Addr = Conf.SecondaryServerAddress
	if Conf.AutoCert {
		Server.AutoTLSManager = autocert.Manager{
			Prompt: autocert.AcceptTOS,
			Cache:  autocert.DirCache(Conf.Certs),
			HostPolicy: func(_ context.Context, h string) error {
				if len(Conf.Whitelist) == 0 || stringsContainsCI(Conf.Whitelist, h) {
					return nil
				}
	
				return fmt.Errorf("acme/autocert: host %q not configured in config.Whitelist", h)
			},
			Email: Conf.MaintainerEmail,
		}
		Server.TLSServer.TLSConfig = Server.AutoTLSManager.TLSConfig()
		Server.TLSServer.Addr = Conf.Address

		Server.Server.Handler = Server.AutoTLSManager.HTTPHandler(nil)
		go Server.Server.ListenAndServe()

		err = Server.StartServer(Server.TLSServer)
	} else {

		Server.Server.Handler = http.HandlerFunc(func(res http.ResponseWriter, req *http.Request) {
			target := "https://"

			if DevMode {
				target += "localhost" + Conf.Address
			} else {
				target += Conf.Domain
			}
			target += req.URL.Path
			if len(req.URL.RawQuery) > 0 {
				target += "?" + req.URL.RawQuery
			}

			http.Redirect(res, req, target, 301)
		})
		go Server.Server.ListenAndServe()

		err = Server.StartTLS(Conf.Address, Conf.TLSCert, Conf.TLSKey)
	}

	if err != nil {
		if time.Since(StartupDate) < time.Second*60 {
			fmt.Println(aurora.Red("unable to start app server, something must be misconfigured: "), err)
		} else {
			fmt.Println(aurora.Red("the server is shutting down now, it's been real: "), err)
		}
	}
}

// LogEntry is a struct containing request logging info
type LogEntry struct {
	Method   string    `json:"method,omitempty"`
	Path     string    `json:"path,omitempty"`
	IP       string    `json:"client,omitempty"`
	Code     int       `json:"code,omitempty"`
	Latency  float64   `json:"latency,omitempty"`
	Start    time.Time `json:"start,omitempty"`
	End      time.Time `json:"end,omitempty"`
	BytesOut int64     `json:"bytesOut,omitempty"`
	BytesIn  int64     `json:"bytesIn,omitempty"`
	DevMode  bool      `json:"devmode,omitempty"`
	Err      string    `json:"err,omitempty"`
	Headers  obj       `json:"headers,omitempty"`
}


// Config holds all the information necessary to fire up a mak instance
type Config struct {
	AppName         string `json:"appname,omitempty" toml:"appname,omitempty"`
	Domain          string `json:"domain,omitempty" toml:"domain,omitempty"`
	MaintainerEmail string `json:"maintainer_email,omitempty" toml:"maintainer_email,omitempty"`

	DevMode bool `json:"devmode,omitempty" toml:"devmode,omitempty"`

	Address                string `json:"address" toml:"address"`
	SecondaryServerAddress string `json:"secondary_server_address" toml:"secondary_server_address"`

	DevAddress                string `json:"dev_address,omitempty" toml:"dev_address,omitempty"`
	DevSecondaryServerAddress string `json:"dev_secondary_server_address,omitempty" toml:"dev_secondary_server_address,omitempty"`

	PreferMsgpack bool `json:"prefer_msgpack,omitempty" toml:"prefer_msgpack,omitempty"`

	AutoPush bool `json:"autopush,omitempty" toml:"autopush,omitempty"`

	AutoCert    bool     `json:"autocert,omitempty" toml:"autocert,omitempty"`
	DevAutoCert bool     `json:"dev_autocert,omitempty" toml:"dev_autocert,omitempty"`
	Whitelist   []string `json:"whitelist,omitempty" toml:"whitelist,omitempty"`
	Certs       string   `json:"certs,omitempty" toml:"certs,omitempty"`

	TLSKey  string `json:"tls_key,omitempty" toml:"tls_key,omitempty"`
	TLSCert string `json:"tls_cert,omitempty" toml:"tls_cert,omitempty"`

	Assets           string `json:"assets,omitempty" toml:"assets,omitempty"`
	DoNotWatchAssets bool   `json:"do_not_watch_assets,omitempty" toml:"do_not_watch_assets,omitempty"`

	Private string `json:"private,omitempty" toml:"private,omitempty"`

	Cache string `json:"cache,omitempty" toml:"cache,omitempty"`

	Raw map[string]interface{} `json:"-" toml:"-"`
}

func digestConfig(location string) *Config {
	raw, err := ioutil.ReadFile(location)
	if err != nil {
		panic("no config file to start with")
	}

	var conf Config
	var rawconf map[string]interface{}

	if strings.Contains(location, ".json") {
		err = jsoniter.Unmarshal(raw, &conf)
		if err == nil {
			err = jsoniter.Unmarshal(raw, &rawconf)
		}
	} else if strings.Contains(location, ".toml") {
		err = toml.Unmarshal(raw, &conf)
		if err == nil {
			err = toml.Unmarshal(raw, &rawconf)
		}
	}

	if err != nil {
		fmt.Println("MakeFromConf err: ", err)
		panic("bad config file, it cannot be parsed. make sure it's valid json or toml")
	}

	conf.Raw = rawconf

	if conf.Private == "" {
		conf.Private = "./private"
	}

	if conf.Assets == "" {
		conf.Assets = "./assets"
	}

	if conf.Certs == "" {
		conf.Certs = conf.Private + "/certs"
	}

	if conf.Cache == "" {
		conf.Cache = conf.Private + "/cache"
	}

	if conf.AutoCert && len(conf.Whitelist) == 0 && conf.Domain != "" {
		conf.Whitelist = []string{conf.Domain}
	}

	return &conf
}
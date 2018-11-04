package backend

import (
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"path/filepath"
	"strings"
	"text/template"
	"net/http"
	"time"

	"github.com/throttled/throttled"
	"github.com/throttled/throttled/store/memstore"
	"github.com/CrowdSurge/banner"
	"github.com/logrusorgru/aurora"
	"github.com/SaulDoesCode/air"
)

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
	// AssetsFolder path to all the servable static assets
	AssetsFolder string
	// AppLocation where this application lives, it's used for self management
	AppLocation string
	// StartupDate when the app started running
	StartupDate time.Time
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

	DevMode = air.DebugMode
	
	AppName = air.Config["app_name"].(string)
	AppDomain = air.Config["domain"].(string)

	AssetsFolder = air.AssetRoot
	air.STATIC("/", air.AssetRoot)
	air.FILE("/", "./assets/index.html")

	if air.MaintainerEmail != "" {
		MaintainerEmails = append(MaintainerEmails, air.MaintainerEmail)
	}

	mailerObj := air.Config["mailer"].(obj)
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

	dbobj := air.Config["db"].(obj)

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

	secretsObj := air.Config["secrets"].(obj)
	tokenSecret := secretsObj["token"].(string)
	verifierSecret := secretsObj["verifier"].(string)
	
	Tokenator = NewBranca(tokenSecret)
	Tokenator.SetTTL(86400 * 7)
	Verinator = NewBranca(verifierSecret)
	Verinator.SetTTL(925)

	initAuth()
	initWrits()

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
	
	air.TheServer.InterceptHandler = func(h http.Handler) http.Handler {
		return httpRateLimiter.RateLimit(h)
	}

	go func() {
		time.Sleep(2 * time.Second)
		fmt.Println("\n", aurora.Green("-------------------------"))
		fmt.Println(aurora.Bold(aurora.Magenta(banner.PrintS(AppName))))
		fmt.Println(aurora.Green("-------------------------"))
	}()

	
	err = air.Serve()
	if err != nil {
		if time.Since(StartupDate) < time.Second*60 {
			fmt.Println(aurora.Red("unable to start app server, something must be misconfigured: "), err)
		} else {
			fmt.Println(aurora.Red("the server is shutting down now, it's been real: "), err)
		}
	}
}

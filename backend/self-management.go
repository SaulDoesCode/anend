package backend

import (
	"fmt"
	"runtime"
	"time"

	"github.com/aofei/air"
)

func buildReplacement() error {
	return exeCC("cd " + AppLocation + ` && go build main.go`)
}

func replaceSelf() (error, error) {
	err := buildReplacement()
	if err != nil {
		fmt.Println("could not build replacement, will run same old executable")
	}
	go air.Shutdown(5 * time.Second)

	return exeC(`nohup bash -c "(sleep 7 && cd ` + AppLocation + ` && sudo ./main) &" &`), err
}

func startSelfManaging() {
	if runtime.GOOS == "windows" {
		if DevMode {
			fmt.Println("there is no support for _updateapp on windows")
		}
		return
	}

	air.GET("/_updateapp", AdminHandle(func(req *air.Request, res *air.Response, user *User) error {
		restartErr, rebuildErr := replaceSelf()

		couldbuild := `<br>the rebuild went fine`
		if rebuildErr != nil {
			couldbuild = `<br>there was a problem with the rebuild: ` + rebuildErr.Error()
		}

		if restartErr != nil {
			return SendHTML(res, 503, `
				<h3>it seems the update command has failed, you'll have to update manually</h3>
				<time>`+time.Now().Format(time.RFC822)+`</time>
				<p>the admin `+user.Username+` is responsible.</p>
				`+couldbuild+`<br>
				restart error: `+restartErr.Error()+`
			`)
		}

		return SendHTML(res, 200, `
			<h3>attempting server update... see you soon.. or not."</h3>
			<time>`+time.Now().Format(time.RFC822)+`</time>
			<p>the admin `+user.Username+` is responsible.</p>
			`+couldbuild+`
		`)
	}))
}

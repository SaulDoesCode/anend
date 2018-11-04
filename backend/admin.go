package backend

import (
	"github.com/SaulDoesCode/air"
)

func initAdmin() {
	air.GET("/admin", AdminHandle(func(req *air.Request, res *air.Response, user *User) error {
		return res.WriteFile(AssetsFolder + "/admin.html")
	}))
}
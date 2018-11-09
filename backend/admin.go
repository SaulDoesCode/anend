package backend

func initAdmin() {
	Mak.GET("/admin", AdminHandle(func(c ctx, user *User) error {
		return c.WriteFile(AssetsDir + "/admin.html")
	}))
}

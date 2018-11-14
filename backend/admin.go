package backend

func initAdmin() {
	Server.GET("/admin", AdminHandle(func(c ctx, user *User) error {
		if Cache != nil {
			return Cache.ServeFile(c.Response(), c.Request(), "/admin.html")
		}
		return c.File("./assets/admin.html")
	}))
}

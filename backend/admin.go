package backend

func initAdmin() {
	Server.GET("/admin", AdminHandle(func(c ctx, user *User) error {
		return c.File("./assets/admin.html")
	}))
}

package backend

func initAdmin() {
	Server.GET("/admin", AdminHandle(func(c ctx, user *User) error {
		if Cache != nil {
			return Cache.ServeFile(c.Response(), c.Request(), "/admin.html")
		}
		return c.File("./assets/admin.html")
	}))

	Server.POST("/query", AdminHandle(func(c ctx, user *User) error {
		var vars obj
		err := c.Bind(&vars)
		if err != nil {
			return c.Msgpack(400, obj{"err": err.Error()})
		}
		query, ok := vars["query"].(string)
		if !ok {
			return c.Msgpack(400, obj{"err": "cannot query without an actual query to query with"})
		}
		delete(vars, "query")

		out, err := Query(query, vars)
		if err != nil {
			return c.Msgpack(500, obj{"err": err.Error()})
		}
		return c.Msgpack(200, out)
	}))
}

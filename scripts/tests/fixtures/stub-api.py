"""A stand-in for the doorcode server's DELETE and GET of a door or user,
for bench-serial-selftest.sh to check bench_env_delete_for_good without a
server. It listens on 127.0.0.1 at a free port, which it writes to the file
named by its one argument, and answers by the first part of the path:

  admin   deletes for good: DELETE answers 204, and GET then 404
  viewer  refuses force with 403, as to a token without the admin role,
          and answers a DELETE without force with 204
  old     ignores force, as a server that does not know it: DELETE answers
          204, and GET 200 with the deleted object
  broken  answers every DELETE with 500

Any request without the header Authorization: Bearer secret is answered 401.
"""

import http.server
import sys

REFUSED = b'{"error":"the admin role is required to delete for good"}'


class Handler(http.server.BaseHTTPRequestHandler):
    def reply(self, status, body=b""):
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def scenario(self):
        if self.headers.get("Authorization") != "Bearer secret":
            self.reply(401, b'{"error":"no valid API token"}')
            return None
        return self.path.split("/")[1]

    def do_DELETE(self):
        scenario = self.scenario()
        forced = self.path.endswith("?force=true")
        if scenario == "broken":
            self.reply(500, b'{"error":"the server failed"}')
        elif scenario == "viewer" and forced:
            self.reply(403, REFUSED)
        elif scenario is not None:
            self.reply(204)

    def do_GET(self):
        scenario = self.scenario()
        if scenario == "old":
            self.reply(200, b'{"id":3,"deleted_at":"2026-11-11T00:00:00Z"}')
        elif scenario is not None:
            self.reply(404, b'{"error":"no such object"}')

    def log_message(self, *_):
        pass


server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
with open(sys.argv[1], "w", encoding="utf-8") as port:
    port.write(str(server.server_port))
server.serve_forever()

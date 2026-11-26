--!nocheck
-- Downloads the minimal Tern backend bundle on first use, so installing the
-- plugin never depends on the desktop app or a hand-set executable path.
local tern = require("tern")
local util = require("./util")
local backend = require("./backend")

local M = {}

local DEFAULT_MANIFEST =
	"https://github.com/zhhpan/literature-reader/releases/latest/download/literature-reader-tern.json"

local function manifestUrl()
	local configured = nil
	local ok, value = pcall(tern.kv.get, "backend_url")
	if ok and type(value) == "string" and value ~= "" then
		configured = value
	end
	local fromEnv = tern.getenv("LITERATURE_READER_BACKEND_URL")
	if type(fromEnv) == "string" and fromEnv ~= "" then
		configured = fromEnv
	end
	return configured or DEFAULT_MANIFEST
end

local function alive(pane)
	for _, entry in ipairs(tern.pane.list()) do
		if entry.pane == pane then
			return true
		end
	end
	return false
end

local function validUrl(url)
	if type(url) ~= "string" then
		return false
	end
	if string.match(url, "^https://[%w.-]+/") then
		return true
	end
	-- A local test server only; release downloads are always HTTPS.
	return string.match(url, "^http://127%.0%.0%.1:%d+/") ~= nil
		or string.match(url, "^http://localhost:%d+/") ~= nil
end

local function validHash(hash)
	return type(hash) == "string" and string.match(hash, "^[0-9a-fA-F]+$") ~= nil and #hash == 64
end

local function setMessage(cx, state, text)
	if not alive(cx.pane) then
		return
	end
	state.backend = state.backend or {}
	state.backend.message = text
	cx:render()
end

local function fail(cx, state, message)
	if not alive(cx.pane) then
		return
	end
	state.backend = {
		state = "error",
		error = message,
		url = manifestUrl(),
	}
	cx:render()
end

-- Runs a program and hands back its result; the callback owns the next step.
local function run(cx, argv, timeout, cb)
	local ok, err = backend.run(argv, { timeout_ms = timeout }, function(result)
		if alive(cx.pane) then
			cb(result)
		end
	end)
	if not ok then
		cb({ status = -1, stdout = "", stderr = tostring(err) })
	end
end

-- certutil prints the path before the digest and "CertUtil" right after it
-- on the next line; the digest line is exactly 64 hex digits (spaced in pairs).
local function hexDigest(text)
	if type(text) ~= "string" then
		return nil
	end
	for line in string.gmatch(text, "[^\r\n]+") do
		local compact = string.gsub(line, "%s+", "")
		if #compact == 64 and string.match(compact, "^%x+$") then
			return compact
		end
		local first = string.match(compact, "^(%x+)")
		if first and #first == 64 then
			return first
		end
	end
	return nil
end

local function verifyZip(cx, state, path, expected, settings, next)
	setMessage(cx, state, "正在校验阅读内核…")
	if settings.windows then
		run(cx, { "certutil.exe", "-hashfile", path, "SHA256" }, 120000, function(result)
			local digest = hexDigest(result.stdout)
			if not digest or string.lower(digest) ~= string.lower(expected) then
				fail(cx, state, "阅读内核校验失败（SHA-256 不匹配）。")
				return
			end
			next()
		end)
	else
		run(cx, { "sha256sum", path }, 120000, function(result)
			local digest = hexDigest(result.stdout)
			if not digest or string.lower(digest) ~= string.lower(expected) then
				fail(cx, state, "阅读内核校验失败（SHA-256 不匹配）。")
				return
			end
			next()
		end)
	end
end

local function extract(cx, state, zip, directory, settings, next)
	setMessage(cx, state, "正在解压阅读内核…")
	local argv = settings.windows
		and { "tar.exe", "-xf", zip, "-C", directory }
		or { "tar", "-xf", zip, "-C", directory }
	run(cx, argv, 300000, function(result)
		if result.status ~= 0 then
			fail(cx, state, "解压阅读内核失败：" .. tostring(result.stderr or ""))
			return
		end
		next()
	end)
end

local function download(cx, state, entry, done)
	local settings = { windows = tern.runtime.os == "windows" }
	local directory = backend.bundleDir()
	local part = directory .. "/backend.zip.part"
	pcall(tern.fs.mkdir, directory)
	util.remove(part)
	setMessage(cx, state, entry.message or "正在下载阅读内核…")
	local url = string.gsub(entry.url, "'", "''")
	local target = string.gsub(part, "'", "''")
	-- PowerShell's .NET TLS stack reaches GitHub release assets where curl's
	-- schannel handshake can stall on some networks; curl stays for other hosts.
	local script = table.concat({
		"$ProgressPreference = 'SilentlyContinue'",
		"[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12",
		"try {",
		"  Invoke-WebRequest -UseBasicParsing -Uri '" .. url .. "' -OutFile '" .. target .. "'",
		"  exit 0",
		"} catch {",
		"  [Console]::Error.WriteLine($_.Exception.Message)",
		"  exit 1",
		"}",
	}, "; ")
	local argv = settings.windows
		and { "powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script }
		or { "curl", "-fsSL", "--retry", "3", "--connect-timeout", "20", "-o", part, entry.url }
	run(cx, argv, 900000, function(result)
		if result.status ~= 0 then
			util.remove(part)
			fail(cx, state, "下载阅读内核失败：" .. tostring(result.stderr or ""))
			return
		end
		verifyZip(cx, state, part, entry.sha256, settings, function()
			extract(cx, state, part, directory, settings, function()
				util.remove(part)
				local exe = backend.bundleExe()
				if not util.exists(exe) then
					fail(cx, state, "下载包缺少阅读内核（literature-reader）。")
					return
				end
				state.exe = exe
				state.backend = { state = "ready" }
				pcall(tern.fs.write, directory .. "/ready.json", tern.json.encode({
					schema = 1,
					version = entry.version,
					sha256 = entry.sha256,
				}, true))
				cx:render()
				done()
			end)
		end)
	end)
end

-- Resolve the bundled backend, downloading it when absent. `done` runs once
-- the backend is ready; failures stay visible in the block for a retry.
function M.ensure(cx, state, done)
	local exe = backend.resolveExe()
	if exe then
		state.exe = exe
		state.backend = nil
		done()
		return
	end
	state.backend = { state = "manifest", message = "正在准备阅读内核…" }
	cx:render()
	tern.fetch(manifestUrl(), { timeout_ms = 20000 }, function(result)
		if not alive(cx.pane) then
			return
		end
		if result.error or result.status ~= 200 then
			fail(cx, state, "无法读取阅读内核清单：" .. tostring(result.error or result.status))
			return
		end
		local entry = util.decode(result.body)
		if type(entry) ~= "table" or not validUrl(entry.url) or not validHash(entry.sha256) then
			fail(cx, state, "阅读内核清单无效。")
			return
		end
		download(cx, state, entry, done)
	end)
end

function M.state(state)
	return state.backend
end

return M

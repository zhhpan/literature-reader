--!nocheck
-- Host half: one native block that renders a bilingual literature snapshot.
local tern = require("tern")
local util = require("./lib/util")
local backend = require("./lib/backend")
local install = require("./lib/install")
local render = require("./lib/render")

local POLL_MS = 600
local LOGIN_POLL_MS = 500
local STATE_CAP = 32 * 1024 * 1024

local R = {}

local function alive(pane)
	for _, entry in ipairs(tern.pane.list()) do
		if entry.pane == pane then
			return true
		end
	end
	return false
end

local function setError(state, message)
	if state.snapshot then
		state.jobError = message
	else
		state.loadError = message
	end
end

local function clampPage(state)
	local snapshot = state.snapshot
	if not snapshot then
		return
	end
	local pages = tonumber(snapshot.pages) or 0
	if pages < 1 then
		state.page = 1
	else
		state.page = util.clamp(state.page or 1, 1, pages)
	end
end

local function refreshSnapshot(state)
	local text = util.readText(state.statePath, STATE_CAP)
	if not text or text == state.lastPayload then
		return false
	end
	local value = util.decode(text)
	if not value then
		return false
	end
	state.lastPayload = text
	if value.error then
		setError(state, tostring(value.error))
	else
		state.snapshot = value
		state.loadError = nil
		state.jobError = nil
		clampPage(state)
	end
	return true
end

local function pumpImages(cx, state)
	if state.imageBusy then
		return
	end
	if not alive(cx.pane) then
		return
	end
	local index = nil
	while true do
		index = table.remove(state.imageQueue or {}, 1)
		if index == nil then
			return
		end
		if state.images[index] == nil and not state.imageFailed[index] then
			break
		end
		index = nil
	end
	state.imageBusy = true
	local out = state.dir .. "/block-" .. tostring(index) .. ".png"
	util.remove(out .. ".error")
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"image",
		state.path,
		"block",
		tostring(index),
		out,
	}, { timeout_ms = 120000 }, function(result)
		state.imageBusy = false
		if not alive(cx.pane) then
			return
		end
		if result.status == 0 then
			local bytes = util.readBytes(out)
			if bytes and #bytes > 0 then
				state.images[index] = cx:blob(bytes, "image/png")
			else
				state.imageFailed[index] = true
			end
		else
			state.imageFailed[index] = true
			local message = util.readText(out .. ".error", 4096)
			if message and message ~= "" then
				state.imageErrors[index] = message
			end
		end
		cx:render()
		pumpImages(cx, state)
	end)
	if not ok then
		state.imageBusy = false
		state.imageFailed[index] = true
		state.imageErrors[index] = tostring(err)
		return
	end
end

local function queueImages(cx, state)
	if not state.snapshot then
		return
	end
	state.imageQueue = state.imageQueue or {}
	state.imageFailed = state.imageFailed or {}
	state.imageErrors = state.imageErrors or {}
	local pending = {}
	for _, index in ipairs(state.imageQueue) do
		pending[index] = true
	end
	for _, block in ipairs(state.snapshot.blocks or {}) do
		local index = block.index
		if block.asset and type(index) == "number" and state.images[index] == nil and not state.imageFailed[index]
			and not pending[index] then
			table.insert(state.imageQueue, index)
			pending[index] = true
		end
	end
	pumpImages(cx, state)
end

local function ensurePage(cx, state, requested)
	if not state.path or state.mode ~= "page" then
		return
	end
	local page = requested
	local pages = state.snapshot and tonumber(state.snapshot.pages) or 0
	if pages > 0 then
		page = util.clamp(page, 1, pages)
	end
	if page < 1 then
		return
	end
	if state.pageBlobs[page] or state.pageFailed[page] then
		return
	end
	if state.pageLoading then
		state.pagePending = page
		return
	end
	state.pageLoading = page
	state.pagePending = nil
	local out = state.dir .. "/page-" .. tostring(page) .. ".png"
	util.remove(out .. ".error")
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"image",
		state.path,
		"page",
		tostring(page),
		out,
	}, { timeout_ms = 120000 }, function(result)
		if state.pageLoading == page then
			state.pageLoading = nil
		end
		if not alive(cx.pane) then
			return
		end
		if result.status == 0 then
			local bytes = util.readBytes(out)
			if bytes and #bytes > 0 then
				state.pageBlobs[page] = cx:blob(bytes, "image/png")
			else
				state.pageFailed[page] = true
			end
		else
			state.pageFailed[page] = true
			state.pageErrors = state.pageErrors or {}
			local message = util.readText(out .. ".error", 4096)
			if message and message ~= "" then
				state.pageErrors[page] = message
			end
		end
		cx:render()
		local nextPage = state.pagePending
		state.pagePending = nil
		if nextPage and state.mode == "page" then
			ensurePage(cx, state, nextPage)
		end
	end)
	if not ok then
		state.pageLoading = nil
		state.pageFailed[page] = true
		state.pageErrors = state.pageErrors or {}
		state.pageErrors[page] = tostring(err)
	end
end

local function startPolling(cx, state)
	state.gen = (state.gen or 0) + 1
	local generation = state.gen
	state.polling = true

	local function tick()
		if generation ~= state.gen then
			return
		end
		if not alive(cx.pane) then
			state.polling = false
			return
		end
		local locked = backend.lockExists(state.statePath)
		local changed = refreshSnapshot(state)
		if changed then
			state.loading = false
			queueImages(cx, state)
			if state.mode == "page" then
				ensurePage(cx, state, state.page)
			end
		end
		if state.cancelling and locked then
			state.cancelTicks = (state.cancelTicks or 0) + 1
			if state.cancelTicks >= 20 then
				util.remove(state.statePath .. ".lock")
				util.remove(state.statePath .. ".cancel")
				state.cancelTicks = 0
				state.job = false
				state.polling = false
				state.attached = false
				state.cancelling = false
				state.jobError = "翻译进程已停止"
				if refreshSnapshot(state) then
					clampPage(state)
					queueImages(cx, state)
				end
				cx:render()
				return
			end
		else
			state.cancelTicks = 0
		end
		if not locked and state.attached then
			state.attached = false
			state.job = false
			state.cancelling = false
			state.polling = false
			if refreshSnapshot(state) then
				clampPage(state)
			end
			cx:render()
			return
		end
		if not locked and not state.job then
			state.polling = false
			if refreshSnapshot(state) then
				clampPage(state)
			end
			cx:render()
			return
		end
		if changed then
			cx:render()
		end
		tern.timer(POLL_MS, tick)
	end

	tern.timer(POLL_MS, tick)
end

function R.autoTranslate(cx, state)
	if state.autoStarted then
		return
	end
	local ok, setting = pcall(tern.kv.get, "auto_translate")
	if not ok or setting ~= true then
		return
	end
	local snapshot = state.snapshot
	local models = state.models
	if not snapshot or not models or models.signed_in ~= true then
		return
	end
	if not snapshot.preferred_model then
		return
	end
	if state.job or state.polling or backend.lockExists(state.statePath) then
		return
	end
	if (snapshot.translated or 0) >= (snapshot.translatable or 0) then
		return
	end
	local translation = snapshot.translation
	local translationState = translation and translation.state
	-- A stopped or failed run is the user's decision, not a reason to retry.
	if translationState == "stopped" or translationState == "error" then
		return
	end
	state.autoStarted = true
	R.startTranslate(cx, state, snapshot.preferred_model)
end

function R.startTranslate(cx, state, model)
	if not state.path then
		return
	end
	if state.job or backend.lockExists(state.statePath) then
		return
	end
	if type(model) ~= "string" or model == "" then
		state.showModels = true
		return
	end
	state.jobError = nil
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"translate",
		state.path,
		state.statePath,
		model,
	}, {}, function(result)
		state.job = false
		state.cancelling = false
		state.cancelTicks = 0
		state.attached = false
		state.gen = (state.gen or 0) + 1
		state.polling = false
		if not alive(cx.pane) then
			return
		end
		refreshSnapshot(state)
		if result.status ~= 0 and not state.jobError and not state.loadError then
			state.jobError = "翻译进程退出码 " .. tostring(result.status)
		end
		cx:render()
	end)
	if not ok then
		state.job = false
		setError(state, backend.startError(err, state.exe))
		cx:render()
		return
	end
	state.job = true
	state.cancelling = false
	state.cancelTicks = 0
	state.attached = false
	startPolling(cx, state)
end

function R.loadModels(cx, state)
	if state.modelsLoading then
		return
	end
	state.modelsLoading = true
	state.modelsError = nil
	local out = state.dir .. "/models.json"
	util.remove(out)
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"models",
		out,
	}, { timeout_ms = 150000 }, function(result)
		state.modelsLoading = false
		if not alive(cx.pane) then
			return
		end
		local value = util.readJson(out)
		if value and value.error then
			state.modelsError = tostring(value.error)
		elseif value then
			state.models = value
			state.modelsError = nil
		else
			state.modelsError = "读取模型列表失败（退出码 " .. tostring(result.status) .. "）"
		end
		R.autoTranslate(cx, state)
		cx:render()
	end)
	if not ok then
		state.modelsLoading = false
		state.modelsError = backend.startError(err, state.exe)
		cx:render()
	end
end

function R.loadRecent(cx, state)
	if state.recentLoading then
		return
	end
	state.recentLoading = true
	state.recentError = nil
	local out = state.dir .. "/list.json"
	util.remove(out)
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"list",
		out,
	}, { timeout_ms = 30000 }, function(result)
		state.recentLoading = false
		if not alive(cx.pane) then
			return
		end
		local value = util.readJson(out)
		if value and value.error then
			state.recentError = tostring(value.error)
		elseif value then
			state.recent = value
			state.recentError = nil
		else
			state.recentError = "读取最近文献失败（退出码 " .. tostring(result.status) .. "）"
		end
		cx:render()
	end)
	if not ok then
		state.recentLoading = false
		state.recentError = backend.startError(err, state.exe)
		cx:render()
	end
end

function R.loadDocument(cx, state)
	state.snapshot = nil
	state.loading = true
	state.loadError = nil
	state.jobError = nil
	state.lastPayload = nil
	state.autoStarted = false
	state.gen = (state.gen or 0) + 1
	state.polling = false
	if backend.lockExists(state.statePath) then
		state.job = true
		state.attached = true
		startPolling(cx, state)
		return
	end
	state.job = false
	state.attached = false
	local out = state.statePath
	util.remove(out)
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"state",
		state.path,
		out,
	}, { timeout_ms = 150000 }, function(result)
		state.loading = false
		if not alive(cx.pane) then
			return
		end
		-- A translate started while this state read was in flight: attach to
		-- its lock instead of trusting a snapshot it is about to overwrite.
		if backend.lockExists(state.statePath) then
			state.job = true
			state.attached = true
			startPolling(cx, state)
			cx:render()
			return
		end
		local value = util.readJson(out)
		if value and value.error then
			setError(state, tostring(value.error))
		elseif value then
			state.snapshot = value
			state.loadError = nil
			state.lastPayload = util.readText(out, STATE_CAP)
			clampPage(state)
			queueImages(cx, state)
			R.autoTranslate(cx, state)
			if state.mode == "page" then
				ensurePage(cx, state, state.page)
			end
		else
			setError(state, "读取文献失败（退出码 " .. tostring(result.status) .. "）")
		end
		cx:render()
	end)
	if not ok then
		state.loading = false
		state.loadError = backend.startError(err, state.exe)
	end
end

function R.openDocument(cx, state, path)
	if type(path) ~= "string" or path == "" then
		return
	end
	if path == state.path then
		R.reload(cx, state)
		return
	end
	if state.job or backend.lockExists(state.statePath) then
		-- The running translate writes this block's shared state.json; stop it
		-- before pointing the block at another document.
		state.cancelling = true
		state.cancelTicks = 0
		pcall(tern.fs.write, state.statePath .. ".cancel", "")
		cx:toast("info", "正在停止当前翻译", "请稍后再打开另一篇文献")
		return
	end
	state.path = path
	state.page = 1
	state.showModels = false
	state.images = {}
	state.imageFailed = {}
	state.imageErrors = {}
	state.imageQueue = {}
	state.imageBusy = false
	state.pageBlobs = {}
	state.pageFailed = {}
	state.pageErrors = {}
	state.pageLoading = nil
	state.pagePending = nil
	R.loadDocument(cx, state)
	if not state.models and not state.modelsLoading then
		R.loadModels(cx, state)
	end
end

function R.reload(cx, state)
	if state.path then
		R.loadDocument(cx, state)
		if not state.models and not state.modelsLoading then
			R.loadModels(cx, state)
		end
	else
		R.loadRecent(cx, state)
	end
end

function R.startLogin(cx, state)
	if state.loginJob then
		return
	end
	state.loginJob = true
	state.loginError = nil
	state.loginPayload = nil
	state.login = { state = "waiting", url = nil }
	util.remove(state.loginPath .. ".cancel")
	local ok, err = backend.run({
		state.exe,
		"--tern",
		"login",
		state.loginPath,
	}, { timeout_ms = 180000 }, function(result)
		state.loginJob = false
		state.loginGen = (state.loginGen or 0) + 1
		if not alive(cx.pane) then
			return
		end
		local value = util.readJson(state.loginPath)
		if value then
			state.login = value
			state.loginPayload = util.readText(state.loginPath, 256 * 1024)
		end
		if (value and (value.state == "done" or value.signed_in == true)) then
			cx:toast("success", "CodeBuddy 登录成功")
			R.loadModels(cx, state)
		elseif value and value.state == "error" then
			cx:toast("error", "登录失败", value.error or nil)
		elseif result.status ~= 0 and not state.loginError then
			state.loginError = "登录进程退出码 " .. tostring(result.status)
		end
		cx:render()
	end)
	if not ok then
		state.loginJob = false
		state.loginError = backend.startError(err, state.exe)
		cx:render()
		return
	end

	state.loginGen = (state.loginGen or 0) + 1
	local generation = state.loginGen
	local function tick()
		if generation ~= state.loginGen or not state.loginJob then
			return
		end
		if not alive(cx.pane) then
			return
		end
		local text = util.readText(state.loginPath, 256 * 1024)
		if text and text ~= state.loginPayload then
			state.loginPayload = text
			local value = util.decode(text)
			if value then
				state.login = value
				cx:render()
			end
		end
		tern.timer(LOGIN_POLL_MS, tick)
	end
	tern.timer(LOGIN_POLL_MS, tick)
end

function R.stopTranslate(cx, state)
	if not state.job and not backend.lockExists(state.statePath) then
		return
	end
	state.cancelling = true
	state.cancelTicks = 0
	local ok, err = pcall(tern.fs.write, state.statePath .. ".cancel", "")
	if not ok then
		state.jobError = "无法写入停止标记：" .. tostring(err)
		cx:render()
		return
	end
	if not state.polling then
		state.job = true
		startPolling(cx, state)
	end
	cx:render()
end

local function toggleMode(cx, state)
	state.mode = state.mode == "page" and "bilingual" or "page"
	if state.mode == "page" then
		ensurePage(cx, state, state.page)
	end
end

local function goPage(cx, state, requested)
	local pages = state.snapshot and tonumber(state.snapshot.pages) or 0
	local page = requested
	if pages > 0 then
		page = util.clamp(page, 1, pages)
	elseif page < 1 then
		page = 1
	end
	state.page = page
	if state.mode == "page" then
		ensurePage(cx, state, page)
	end
end

local reader = {
	font = "sans",

	init = function(cx, args, saved)
		local dir = tern.plugin.data .. "/panes/" .. tostring(cx.pane)
		pcall(tern.fs.mkdir, dir)
		local exe = backend.resolveExe()

		local state = {
			path = saved and saved.path or nil,
			snapshot = nil,
			models = nil,
			loading = false,
			loadError = nil,
			exe = exe,
			backend = nil,
			dir = dir,
			mode = (saved and (saved.mode == "page" and "page" or "bilingual")) or "bilingual",
			page = (saved and tonumber(saved.page)) or 1,
			showZh = not (saved and saved.showZh == false),
			showModels = false,
			login = nil,
			loginJob = false,
			job = false,
			cancelling = false,
			cancelTicks = 0,
			polling = false,
			attached = false,
			images = {},
			imageFailed = {},
			imageErrors = {},
			imageQueue = {},
			imageBusy = false,
			pageBlobs = {},
			pageFailed = {},
			pageErrors = {},
			pageLoading = nil,
			pagePending = nil,
			lastPayload = nil,
			gen = 0,
			recent = nil,
			recentLoading = false,
			recentError = nil,
			modelsLoading = false,
			modelsError = nil,
			jobError = nil,
			loginGen = 0,
			loginPayload = nil,
			loginError = nil,
			autoStarted = false,
			model = nil,
			statePath = dir .. "/state.json",
			modelsPath = dir .. "/models.json",
			loginPath = dir .. "/login.json",
		}

		local fromArgs = args and args[1]
		if type(fromArgs) == "string" and fromArgs ~= "" then
			state.path = fromArgs
		end
		if state.page < 1 then
			state.page = 1
		end

		local function start()
			if state.path then
				R.loadModels(cx, state)
				R.loadDocument(cx, state)
			else
				R.loadRecent(cx, state)
			end
			if tern.kv.get("login_requested") == true then
				tern.kv.set("login_requested", nil)
				R.startLogin(cx, state)
			end
		end
		if state.exe then
			start()
		else
			state.backend = { state = "manifest", message = "正在准备阅读内核…" }
			install.ensure(cx, state, start)
		end
		return state
	end,

	title = function(state)
		local title = state.snapshot and state.snapshot.title
		if type(title) ~= "string" or title == "" then
			title = state.path and util.basename(state.path) or nil
		end
		if type(title) ~= "string" or title == "" then
			return "文献阅读器"
		end
		return util.truncate(title, 60)
	end,

	view = function(state, cx)
		return render.view(state)
	end,

	key = function(state, key, cx)
		if key.ctrl or key.alt or key.meta then
			return false
		end
		local name = key.name
		if name == "t" then
			state.showZh = not state.showZh
			return true
		elseif name == "p" then
			if not state.path or not state.snapshot then
				return false
			end
			toggleMode(cx, state)
			return true
		elseif name == "[" then
			if state.mode ~= "page" or not state.snapshot then
				return false
			end
			goPage(cx, state, state.page - 1)
			return true
		elseif name == "]" then
			if state.mode ~= "page" or not state.snapshot then
				return false
			end
			goPage(cx, state, state.page + 1)
			return true
		elseif name == "r" then
			R.reload(cx, state)
			return true
		end
		return false
	end,

	event = function(state, ev, cx)
		if ev.ev ~= "action" then
			return
		end
		local act = ev.act
		local value = ev.value
		if act == "translate" then
			if not state.path or not state.snapshot then
				R.loadDocument(cx, state)
				return
			end
			local model = state.model or state.snapshot.model_id or state.snapshot.preferred_model
			if not model then
				state.showModels = true
			else
				R.startTranslate(cx, state, model)
			end
		elseif act == "stop" then
			R.stopTranslate(cx, state)
		elseif act == "models" then
			state.showModels = true
		elseif act == "models-retry" then
			R.loadModels(cx, state)
		elseif act == "zh" then
			state.showZh = not state.showZh
		elseif act == "close-models" then
			state.showModels = false
		elseif act == "model" then
			if type(value) == "string" and value ~= "" then
				state.model = value
				state.showModels = false
				R.startTranslate(cx, state, value)
			end
		elseif act == "login" then
			R.startLogin(cx, state)
		elseif act == "copy-url" then
			local login = state.login
			if login and login.url and login.url ~= "" then
				cx:copy(login.url)
				cx:toast("success", "登录链接已复制")
			end
		elseif act == "mode" then
			if state.snapshot then
				toggleMode(cx, state)
			end
		elseif act == "page" then
			local requested = tonumber(value)
			if requested then
				goPage(cx, state, math.floor(requested))
			end
		elseif act == "open" then
			R.openDocument(cx, state, value)
		elseif act == "backend-retry" then
			install.ensure(cx, state, function()
				if state.path then
					R.loadModels(cx, state)
					R.loadDocument(cx, state)
				else
					R.loadRecent(cx, state)
				end
			end)
		elseif act == "reload" then
			R.reload(cx, state)
		end
	end,

	save = function(state)
		return {
			path = state.path,
			mode = state.mode,
			page = state.page,
			showZh = state.showZh,
		}
	end,
}

tern.block.define("reader", reader)

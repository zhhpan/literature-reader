--!nocheck
-- Locating and starting the Rust binary that serves the JSON backend.
local tern = require("tern")
local util = require("./util")

local M = {}

local WINDOWS = tern.runtime.os == "windows"
local EXE_NAME = WINDOWS and "literature-reader.exe" or "literature-reader"

local function join(a, b)
	if not a or a == "" then
		return b
	end
	local last = string.sub(a, -1)
	if last == "/" or last == "\\" then
		return a .. b
	end
	return a .. "/" .. b
end

M.EXE_NAME = EXE_NAME

-- Every candidate in resolution order: { value, verify }.
-- The bare PATH fallback is not verified (tern.fs.exists cannot search PATH).
function M.candidates()
	local list = {}

	local configured = nil
	local ok, value = pcall(tern.kv.get, "exe")
	if ok and type(value) == "string" and value ~= "" then
		configured = value
		table.insert(list, { path = configured, verify = true, source = "kv" })
	end

	local fromEnv = tern.getenv("LITERATURE_READER_EXE")
	if fromEnv and fromEnv ~= "" and fromEnv ~= configured then
		table.insert(list, { path = fromEnv, verify = true, source = "env" })
	end

	if WINDOWS then
		local localAppData = tern.getenv("LOCALAPPDATA")
		if localAppData and localAppData ~= "" then
			table.insert(list, {
				path = join(join(localAppData, "Programs"), join("Literature Reader", EXE_NAME)),
				verify = true,
				source = "install",
			})
		end
	end

	local dir = tern.plugin.dir
	if dir and dir ~= "" then
		table.insert(list, {
			path = join(join(join(dir, ".."), ".."), join("target/release", EXE_NAME)),
			verify = true,
			source = "target/release",
		})
		table.insert(list, {
			path = join(join(join(dir, ".."), ".."), join("target/debug", EXE_NAME)),
			verify = true,
			source = "target/debug",
		})
	end

	table.insert(list, { path = EXE_NAME, verify = false, source = "PATH" })
	return list
end

-- One resolution per block. The first existing candidate wins; when none
-- exists the bare program name is returned so the PATH lookup can try.
function M.resolveExe()
	local candidates = M.candidates()
	for _, candidate in ipairs(candidates) do
		if candidate.verify then
			if util.exists(candidate.path) then
				return candidate.path
			end
		else
			return candidate.path
		end
	end
	return nil
end

function M.lockExists(statePath)
	return util.exists(statePath .. ".lock")
end

-- A readable diagnosis for tern.process.run's spawn error.
function M.startError(err, exe)
	local message = tostring(err or "未知错误")
	return string.format(
		"无法启动后端程序 %s：%s\n\n请先安装「文献阅读器」，或在命令面板运行「文献阅读器：设置可执行文件…」指定 literature-reader 的完整路径。",
		tostring(exe or EXE_NAME),
		message
	)
end

-- pcall wrapper: returns true when the process started, false plus the error.
function M.run(argv, opts, cb)
	local ok, err = pcall(tern.process.run, argv, opts, cb)
	if ok then
		return true, nil
	end
	return false, err
end

return M

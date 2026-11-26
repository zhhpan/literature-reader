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

-- The plugin's own runtime. Downloading it here keeps the Tern plugin and the
-- desktop app independent: neither needs the other's executable.
function M.bundleDir()
	return join(tern.plugin.data, "backend")
end

function M.bundleExe()
	return join(M.bundleDir(), EXE_NAME)
end

-- The on-demand layout package carries a layout-capable backend under a
-- separate name so it never replaces a locked executable in place.
function M.layoutExe()
	return join(M.bundleDir(), "literature-reader-layout.exe")
end

-- Resolution order. The bundled runtime is the normal path; kv/env are
-- developer overrides, never a setup step.
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

	table.insert(list, { path = M.layoutExe(), verify = true, source = "layout" })
	table.insert(list, { path = M.bundleExe(), verify = true, source = "bundle" })

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

	return list
end

-- One resolution per block. The first existing candidate wins; nil means the
-- plugin must prepare (download) its bundled backend.
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

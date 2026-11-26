// dawsu! isolated VST3 instrument & effect host (Protocol v4).
//
// One process hosts one VST3 plug-in (instrument or effect), so a crashing plug-in only takes down its own track/node.
// Built directly on the Steinberg VST 3 SDK (MIT licence since 3.8); no JUCE.
//
//   dawsu_vst3_host --scan <plugin.vst3>
//       Prints one line per instrument or effect class (canonical cross-OS TUID):
//       CLASS \t <uid> \t <name> \t <vendor> \t <subcategories>; exits 0.
//   dawsu_vst3_host --host <name> <parentPid> <plugin.vst3> [<uid>] [--offline] [--sample-rate <hz>]
//       Loads the plug-in and prints READY \t <name> \t <protocol> (or ERROR \t <message>) on stdout. Then:
//       - Audio: the shared memory "Local\<name>" (Magic "DVH4", Protocol 4, atomic header + ProcessContext).
//         dawsu! fills events, optional stereo audio input (inLeft/inRight + ShmFlagHasAudioIn), and frames,
//         bumps requestSeq (release) and signals "Local\<name>-req"; the host renders, updates latency/tail,
//         sets doneSeq (release) and signals "Local\<name>-done".
//       - Control: "<id> \t <command>" per stdin line, "<id> \t <reply>" per stdout line:
//         SHOW / HIDE, GETSTATE -> STATE \t <base64>, SETSTATE \t <base64> -> OK,
//         SET_BYPASS \t <0|1> -> OK, NOTENAMES -> NAMES \t ..., RENDER \t ..., QUIT.

#include "public.sdk/source/vst/hosting/module.h"
#include "public.sdk/source/vst/hosting/plugprovider.h"
#include "public.sdk/source/vst/hosting/hostclasses.h"
#include "public.sdk/source/vst/hosting/eventlist.h"
#include "public.sdk/source/vst/hosting/parameterchanges.h"
#include "public.sdk/source/vst/hosting/processdata.h"
#include "public.sdk/source/common/memorystream.h"
#include "pluginterfaces/gui/iplugview.h"
#include "pluginterfaces/gui/iplugviewcontentscalesupport.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivstmidicontrollers.h"
#include "pluginterfaces/vst/ivstunits.h"
#include "pluginterfaces/vst/ivstprocesscontext.h"

#define NOMINMAX
#include <windows.h>
#include <avrt.h>
#include <wincrypt.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

using namespace Steinberg;
using namespace Steinberg::Vst;

namespace {

// ---- Shared memory: mirrored by dawsu-audio-instruments (keep the two in step) ----
constexpr uint32_t Magic = 0x34485644; // "DVH4"
constexpr int Protocol = 4;
constexpr int MaxEvents = 1024, MaxFrames = 4096;
constexpr uint32_t ShmFlagHasAudioIn = 1u << 0;
constexpr uint32_t ShmFlagBypassed = 1u << 1;

struct ShmEvent { int32_t offset; uint8_t status, data1, data2, pad; };
struct ShmProcessContext
{
    uint32_t stateFlags;
    int32_t timeSigNum;
    int32_t timeSigDen;
    uint32_t pad;
    double sampleRate;
    double tempo;
    int64_t projectTimeSamples;
    double projectTimeMusic;
    double barPositionMusic;
    double cycleStartMusic;
    double cycleEndMusic;
};
struct Shm
{
    uint32_t magic, version;
    int32_t frames, eventCount;
    std::atomic<uint32_t> requestSeq, doneSeq;
    std::atomic<uint32_t> status, flags;
    std::atomic<uint32_t> latencySamples, tailSamples;
    std::atomic<uint32_t> restartFlags;
    uint32_t reserved;
    ShmProcessContext context;
    ShmEvent events[MaxEvents];
    float inLeft[MaxFrames], inRight[MaxFrames];
    float left[MaxFrames], right[MaxFrames];
};
static_assert(sizeof(std::atomic<uint32_t>) == 4, "atomic u32 size");
static_assert(sizeof(ShmEvent) == 8, "event layout");
static_assert(sizeof(ShmProcessContext) == 72, "context layout");
static_assert(
    offsetof(Shm, requestSeq) == 16 &&
    offsetof(Shm, doneSeq) == 20 &&
    offsetof(Shm, status) == 24 &&
    offsetof(Shm, flags) == 28 &&
    offsetof(Shm, latencySamples) == 32 &&
    offsetof(Shm, tailSamples) == 36 &&
    offsetof(Shm, restartFlags) == 40 &&
    offsetof(Shm, context) == 48 &&
    offsetof(Shm, events) == 120 &&
    offsetof(Shm, inLeft) == 120 + MaxEvents * 8 &&
    offsetof(Shm, inRight) == 120 + MaxEvents * 8 + MaxFrames * sizeof(float) &&
    offsetof(Shm, left) == 120 + MaxEvents * 8 + MaxFrames * sizeof(float) * 2 &&
    offsetof(Shm, right) == 120 + MaxEvents * 8 + MaxFrames * sizeof(float) * 3,
    "shared memory layout");

constexpr size_t MaxStateBytes = 64u * 1024 * 1024;
std::mutex outLock;
HANDLE protocolOut = INVALID_HANDLE_VALUE;
void Reply(const std::string& line)
{
    std::lock_guard<std::mutex> lock(outLock);
    std::string text = line + "\n";
    DWORD written = 0;
    for (size_t at = 0; at < text.size(); at += written)
        if (!WriteFile(protocolOut, text.data() + at, (DWORD)(text.size() - at), &written, nullptr) || written == 0) return;
}

void IsolateStdout()
{
    DuplicateHandle(GetCurrentProcess(), GetStdHandle(STD_OUTPUT_HANDLE), GetCurrentProcess(), &protocolOut, 0, FALSE, DUPLICATE_SAME_ACCESS);
    HANDLE nul = CreateFileW(L"NUL", GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, 0, nullptr);
    SetStdHandle(STD_OUTPUT_HANDLE, nul);
    FILE* ignored = nullptr;
    freopen_s(&ignored, "NUL", "w", stdout);
}

std::string Clean(std::string s)
{
    for (auto& c : s) if (c == '\t' || c == '\n' || c == '\r') c = ' ';
    return s;
}

std::string NormalizeHexUid(const std::string& s)
{
    std::string out;
    out.reserve(32);
    for (unsigned char c : s)
    {
        if (std::isxdigit(c)) out.push_back((char)std::toupper(c));
    }
    return out;
}

bool UidMatches(const VST3::UID& cid, const std::string& wanted)
{
    std::string normWanted = NormalizeHexUid(wanted);
    if (normWanted.empty()) return false;
    return NormalizeHexUid(cid.toString(false)) == normWanted ||
           NormalizeHexUid(cid.toString(true)) == normWanted;
}

// ---- Base64 for state blobs on the text control channel (Windows' crypt32) ----
std::string ToBase64(const std::vector<uint8_t>& in)
{
    if (in.empty()) return "";
    DWORD n = 0;
    const DWORD flags = CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF;
    if (!CryptBinaryToStringA(in.data(), (DWORD)in.size(), flags, nullptr, &n)) return "";
    std::string out(n, '\0');
    if (!CryptBinaryToStringA(in.data(), (DWORD)in.size(), flags, out.data(), &n)) return "";
    out.resize(n);
    return out;
}

bool FromBase64(const std::string& in, size_t from, std::vector<uint8_t>& out)
{
    DWORD n = 0;
    const char* text = in.data() + from;
    DWORD length = (DWORD)(in.size() - from);
    if (length == 0) { out.clear(); return true; }
    if (!CryptStringToBinaryA(text, length, CRYPT_STRING_BASE64, nullptr, &n, nullptr, nullptr) || n > MaxStateBytes) return false;
    out.resize(n);
    return CryptStringToBinaryA(text, length, CRYPT_STRING_BASE64, out.data(), &n, nullptr, nullptr) && (out.resize(n), true);
}

// ---- Hosted plug-in globals & state ----
VST3::Hosting::Module::Ptr module;
IPtr<PlugProvider> provider;
IPtr<IComponent> component;
IPtr<IAudioProcessor> processor;
IPtr<IEditController> controller;
IPtr<IMidiMapping> midiMapping;
HostApplication hostApp;
std::string pluginName;
std::mutex processLock;
Shm* globalShm = nullptr;

std::array<std::atomic<ParamID>, 16 * kCountCtrlNumber> midiMap;
ParamID bypassParamId = kNoParamId;
std::atomic<bool> hostBypassed{false};
std::atomic<bool> pendingLatencyReactivate{false};

bool IsSingleComponent()
{
    if (!component || !controller) return false;
    FUnknown* compUnk = nullptr;
    FUnknown* ctrlUnk = nullptr;
    if (component->queryInterface(FUnknown::iid, (void**)&compUnk) == kResultOk && compUnk) compUnk->release();
    if (controller->queryInterface(FUnknown::iid, (void**)&ctrlUnk) == kResultOk && ctrlUnk) ctrlUnk->release();
    return compUnk && compUnk == ctrlUnk;
}

void ReadMidiMap()
{
    for (auto& slot : midiMap) slot.store(kNoParamId, std::memory_order_relaxed);
    if (!midiMapping) return;
    for (int16 channel = 0; channel < 16; channel++)
        for (int16 cc = 0; cc < kCountCtrlNumber; cc++)
        {
            ParamID id = kNoParamId;
            if (midiMapping->getMidiControllerAssignment(0, channel, (CtrlNumber)cc, id) == kResultTrue)
                midiMap[channel * kCountCtrlNumber + cc].store(id, std::memory_order_relaxed);
        }
}

void DiscoverBypassParam()
{
    bypassParamId = kNoParamId;
    if (!controller) return;
    int32 count = controller->getParameterCount();
    for (int32 i = 0; i < count; i++)
    {
        ParameterInfo info{};
        if (controller->getParameterInfo(i, info) == kResultOk &&
            (info.flags & ParameterInfo::kIsBypass) != 0)
        {
            bypassParamId = info.id;
            break;
        }
    }
}

void RefreshLatencyAndTail()
{
    if (!globalShm || !processor) return;
    uint32_t lat = processor->getLatencySamples();
    uint32_t tail = processor->getTailSamples();
    globalShm->latencySamples.store(lat, std::memory_order_release);
    globalShm->tailSamples.store(tail, std::memory_order_release);
}

// ---- Host-side interfaces the plug-in calls ----
constexpr size_t MaxPendingEdits = 4096;

class ComponentHandler : public IComponentHandler
{
public:
    std::mutex lock;
    std::vector<std::pair<ParamID, ParamValue>> pending;

    ComponentHandler() { pending.reserve(MaxPendingEdits); }

    tresult PLUGIN_API beginEdit(ParamID) override { return kResultOk; }
    tresult PLUGIN_API performEdit(ParamID id, ParamValue value) override
    {
        std::lock_guard<std::mutex> g(lock);
        for (auto& edit : pending)
            if (edit.first == id) { edit.second = value; return kResultOk; }
        if (pending.size() < MaxPendingEdits) pending.emplace_back(id, value);
        return kResultOk;
    }
    tresult PLUGIN_API endEdit(ParamID) override { return kResultOk; }
    tresult PLUGIN_API restartComponent(int32 flags) override
    {
        if (globalShm)
            globalShm->restartFlags.fetch_or((uint32_t)flags, std::memory_order_acq_rel);
        if (flags & kLatencyChanged)
            pendingLatencyReactivate.store(true, std::memory_order_release);
        if (flags & kMidiCCAssignmentChanged)
            ReadMidiMap();
        return kResultOk;
    }
    tresult PLUGIN_API queryInterface(const TUID iid, void** obj) override
    {
        if (FUnknownPrivate::iidEqual(iid, IComponentHandler::iid) || FUnknownPrivate::iidEqual(iid, FUnknown::iid))
        {
            *obj = this;
            return kResultOk;
        }
        *obj = nullptr;
        return kNoInterface;
    }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
};

HWND editorWindow = nullptr;
IPtr<IPlugView> view;

class PlugFrame : public IPlugFrame
{
public:
    tresult PLUGIN_API resizeView(IPlugView* v, ViewRect* r) override
    {
        if (!editorWindow || !r) return kInvalidArgument;
        RECT rect{0, 0, r->getWidth(), r->getHeight()};
        AdjustWindowRect(&rect, GetWindowLong(editorWindow, GWL_STYLE), FALSE);
        SetWindowPos(editorWindow, nullptr, 0, 0, rect.right - rect.left, rect.bottom - rect.top, SWP_NOMOVE | SWP_NOZORDER);
        return v ? v->onSize(r) : kResultOk;
    }
    tresult PLUGIN_API queryInterface(const TUID iid, void** obj) override
    {
        if (FUnknownPrivate::iidEqual(iid, IPlugFrame::iid) || FUnknownPrivate::iidEqual(iid, FUnknown::iid))
        {
            *obj = this;
            return kResultOk;
        }
        *obj = nullptr;
        return kNoInterface;
    }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
};

ComponentHandler handler;
PlugFrame frame;

bool IsSupportedAudioClass(const VST3::Hosting::ClassInfo& c)
{
    if (c.category() != kVstAudioEffectClass) return false;
    const auto& sub = c.subCategoriesString();
    return sub.find("Instrument") != std::string::npos || sub.find("Fx") != std::string::npos;
}

int Scan(const std::string& path)
{
    std::string error;
    auto m = VST3::Hosting::Module::create(path, error);
    if (!m) { Reply("ERROR\t" + Clean(error)); return 2; }
    for (auto& c : m->getFactory().classInfos())
        if (IsSupportedAudioClass(c))
            Reply("CLASS\t" + c.ID().toString(false) + "\t" + Clean(c.name()) + "\t" + Clean(c.vendor()) + "\t" + Clean(c.subCategoriesString()));
    return 0;
}

bool offline = false;
double configuredSampleRate = 44100.0;

bool ConfigureProcessing(double sampleRate, std::string& error)
{
    if (!(sampleRate > 0.0) || !std::isfinite(sampleRate)) sampleRate = 44100.0;
    ProcessSetup setup{offline ? kOffline : kRealtime, kSample32, MaxFrames, sampleRate};
    if (processor->setupProcessing(setup) != kResultOk)
    {
        error = "The plug-in refused " + std::to_string((int)std::llround(sampleRate)) + " Hz processing.";
        return false;
    }
    if (component->setActive(true) != kResultOk) { error = "The plug-in couldn't be activated."; return false; }
    processor->setProcessing(true);
    configuredSampleRate = sampleRate;
    RefreshLatencyAndTail();
    return true;
}

bool Load(const std::string& path, const std::string& uid, double initialSampleRate, std::string& error)
{
    module = VST3::Hosting::Module::create(path, error);
    if (!module) return false;
    auto factory = module->getFactory();
    for (auto& c : factory.classInfos())
    {
        if (c.category() != kVstAudioEffectClass) continue;
        if (uid.empty() ? !IsSupportedAudioClass(c) : !UidMatches(c.ID(), uid)) continue;
        provider = owned(new PlugProvider(factory, c, true));
        if (!provider->initialize()) { provider = nullptr; continue; }
        pluginName = c.name();
        break;
    }
    if (!provider) { error = "No VST3 instrument or effect class was found in this plug-in."; return false; }

    component = provider->getComponentPtr();
    controller = provider->getControllerPtr();
    processor = FUnknownPtr<IAudioProcessor>(component);
    if (!component || !processor) { error = "The plug-in has no audio processor."; return false; }
    if (controller)
    {
        controller->setComponentHandler(&handler);
        midiMapping = FUnknownPtr<IMidiMapping>(controller);
    }

    int32 outputs = component->getBusCount(kAudio, kOutput), inputs = component->getBusCount(kAudio, kInput);
    if (outputs < 1) { error = "The plug-in has no audio output."; return false; }
    std::vector<SpeakerArrangement> outArr(outputs, SpeakerArr::kStereo), inArr(inputs, SpeakerArr::kStereo);
    processor->setBusArrangements(inputs ? inArr.data() : nullptr, inputs, outArr.data(), outputs);
    for (int32 i = 0; i < outputs; i++) component->activateBus(kAudio, kOutput, i, i == 0);
    for (int32 i = 0; i < inputs; i++) component->activateBus(kAudio, kInput, i, i == 0);
    if (component->getBusCount(kEvent, kInput) > 0) component->activateBus(kEvent, kInput, 0, true);

    if (!ConfigureProcessing(initialSampleRate, error)) return false;
    ReadMidiMap();
    DiscoverBypassParam();
    return true;
}

// ---- State: component state, then controller state, each with a 4-byte length ----
bool ReadStream(MemoryStream& s, std::vector<uint8_t>& out)
{
    int64 size = 0;
    if (s.seek(0, IBStream::kIBSeekEnd, &size) != kResultOk || size < 0 || size > (int64)MaxStateBytes) return false;
    s.seek(0, IBStream::kIBSeekSet, nullptr);
    out.resize((size_t)size);
    int32 read = 0;
    return size == 0 || (s.read(out.data(), (int32)size, &read) == kResultOk && read == size);
}

std::string GetState()
{
    std::vector<uint8_t> a, b, all;
    bool singleComp = IsSingleComponent();
    {
        std::lock_guard<std::mutex> g(processLock);
        MemoryStream comp, ctrl;
        if (component->getState(&comp) != kResultOk || !ReadStream(comp, a)) return "ERROR\tThe plug-in didn't save its state.";
        if (controller && !singleComp && controller->getState(&ctrl) == kResultOk && !ReadStream(ctrl, b))
            return "ERROR\tThe plug-in's editor state couldn't be read.";
    }
    if (a.size() + b.size() > MaxStateBytes) return "ERROR\tThe plug-in's state is larger than 64 MB.";
    all.reserve(1 + 8 + a.size() + b.size());
    all.push_back(1);
    for (auto* part : {&a, &b})
    {
        uint32_t n = (uint32_t)part->size();
        for (int i = 0; i < 4; i++) all.push_back((uint8_t)(n >> (8 * i)));
        all.insert(all.end(), part->begin(), part->end());
    }
    return "STATE\t" + ToBase64(all);
}

std::string SetState(const std::string& line, size_t from)
{
    std::vector<uint8_t> all;
    if (!FromBase64(line, from, all) || all.empty() || all[0] != 1) return "ERROR\tThe saved state is damaged or from a newer dawsu!.";
    std::vector<uint8_t> parts[2];
    size_t pos = 1;
    for (auto& part : parts)
    {
        if (pos + 4 > all.size()) return "ERROR\tThe saved state is damaged.";
        uint32_t n = all[pos] | all[pos + 1] << 8 | all[pos + 2] << 16 | (uint32_t)all[pos + 3] << 24;
        pos += 4;
        if (n > all.size() - pos) return "ERROR\tThe saved state is damaged.";
        part.assign(all.begin() + pos, all.begin() + pos + n);
        pos += n;
    }

    bool singleComp = IsSingleComponent();
    std::lock_guard<std::mutex> g(processLock);
    processor->setProcessing(false);
    component->setActive(false);

    MemoryStream comp((void*)parts[0].data(), (TSize)parts[0].size()), ctrl((void*)parts[1].data(), (TSize)parts[1].size());
    if (component->setState(&comp) != kResultOk)
    {
        component->setActive(true);
        processor->setProcessing(true);
        return "ERROR\tThe plug-in rejected its saved state.";
    }
    if (controller && !singleComp)
    {
        comp.seek(0, IBStream::kIBSeekSet, nullptr);
        controller->setComponentState(&comp);
        if (!parts[1].empty()) controller->setState(&ctrl);
    }

    component->setActive(true);
    processor->setProcessing(true);
    RefreshLatencyAndTail();
    return "OK";
}

// ---- DPI Scaling + Editor window (on the main thread) ----
float GetWindowScale(HWND hwnd)
{
    using GetDpiForWindowFn = UINT(WINAPI*)(HWND);
    using GetDpiForSystemFn = UINT(WINAPI*)();
    HMODULE user32 = GetModuleHandleW(L"user32.dll");
    if (hwnd && user32)
    {
        auto fn = reinterpret_cast<GetDpiForWindowFn>(GetProcAddress(user32, "GetDpiForWindow"));
        if (fn)
        {
            UINT dpi = fn(hwnd);
            if (dpi > 0) return (float)dpi / 96.0f;
        }
    }
    if (user32)
    {
        auto fnSys = reinterpret_cast<GetDpiForSystemFn>(GetProcAddress(user32, "GetDpiForSystem"));
        if (fnSys)
        {
            UINT dpi = fnSys();
            if (dpi > 0) return (float)dpi / 96.0f;
        }
    }
    return 1.0f;
}

void ApplyViewScaleFactor(float scale)
{
    if (!view || !(scale > 0.0f) || !std::isfinite(scale)) return;
    FUnknownPtr<IPlugViewContentScaleSupport> scaleSupport(view);
    if (scaleSupport) scaleSupport->setContentScaleFactor(scale);
}

LRESULT CALLBACK EditorProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    if (msg == WM_CLOSE)
    {
        ShowWindow(hwnd, SW_HIDE);
        return 0;
    }
    if (msg == WM_DPICHANGED && lp)
    {
        float scale = (float)HIWORD(wp) / 96.0f;
        ApplyViewScaleFactor(scale);
        auto* suggested = reinterpret_cast<const RECT*>(lp);
        SetWindowPos(hwnd, nullptr, suggested->left, suggested->top,
            suggested->right - suggested->left, suggested->bottom - suggested->top,
            SWP_NOZORDER | SWP_NOACTIVATE);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

std::string ShowEditor()
{
    if (!controller) return "ERROR\tThe plug-in has no editor.";
    if (!editorWindow)
    {
        view = owned(controller->createView(ViewType::kEditor));
        if (!view || view->isPlatformTypeSupported(kPlatformTypeHWND) != kResultTrue) { view = nullptr; return "ERROR\tThe plug-in has no editor window."; }
        float initialScale = GetWindowScale(nullptr);
        ApplyViewScaleFactor(initialScale);
        ViewRect r{0, 0, 800, 600};
        view->getSize(&r);
        RECT rect{0, 0, r.getWidth(), r.getHeight()};
        DWORD style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        AdjustWindowRect(&rect, style, FALSE);
        std::wstring title(MultiByteToWideChar(CP_UTF8, 0, pluginName.data(), (int)pluginName.size(), nullptr, 0), L'\0');
        MultiByteToWideChar(CP_UTF8, 0, pluginName.data(), (int)pluginName.size(), title.data(), (int)title.size());
        title += L" — dawsu!";
        editorWindow = CreateWindowW(L"DawsuVst3Editor", title.c_str(), style, CW_USEDEFAULT, CW_USEDEFAULT,
            rect.right - rect.left, rect.bottom - rect.top, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
        view->setFrame(&frame);
        float windowScale = GetWindowScale(editorWindow);
        if (std::fabs(windowScale - initialScale) > 0.01f) ApplyViewScaleFactor(windowScale);
        if (view->attached(editorWindow, kPlatformTypeHWND) != kResultTrue)
        {
            DestroyWindow(editorWindow);
            editorWindow = nullptr;
            view = nullptr;
            return "ERROR\tThe plug-in's editor didn't open.";
        }
    }
    ShowWindow(editorWindow, SW_SHOW);
    SetForegroundWindow(editorWindow);
    return "OK";
}

std::string Render(const std::string& line);

std::string NoteNames()
{
    std::string reply = "NAMES";
    FUnknownPtr<IUnitInfo> units(controller);
    if (!units) return reply;
    for (int32 l = 0; l < units->getProgramListCount(); l++)
    {
        ProgramListInfo list{};
        if (units->getProgramListInfo(l, list) != kResultOk) continue;
        for (int32 program = 0; program < std::max<int32>(1, list.programCount); program++)
        {
            if (units->hasProgramPitchNames(list.id, program) != kResultTrue) continue;
            for (int16 pitch = 0; pitch < 128; pitch++)
            {
                String128 name{};
                if (units->getProgramPitchName(list.id, program, pitch, name) != kResultOk || !name[0]) continue;
                int length = 0;
                while (length < 128 && name[length]) length++;
                auto* wide = reinterpret_cast<LPCWCH>(name);
                int bytes = WideCharToMultiByte(CP_UTF8, 0, wide, length, nullptr, 0, nullptr, nullptr);
                std::string utf8(bytes > 0 ? bytes : 0, '\0');
                WideCharToMultiByte(CP_UTF8, 0, wide, length, utf8.data(), bytes, nullptr, nullptr);
                reply += "\t" + std::to_string(pitch) + "=" + Clean(utf8);
            }
            return reply;
        }
    }
    return reply;
}

std::string Run(const std::string& line)
{
    if (line == "SHOW") return ShowEditor();
    if (line == "HIDE") { if (editorWindow) ShowWindow(editorWindow, SW_HIDE); return "OK"; }
    if (line == "GETSTATE") return GetState();
    if (line == "NOTENAMES") return NoteNames();
    if (line.rfind("SETSTATE\t", 0) == 0) return SetState(line, 9);
    if (line.rfind("SET_BYPASS\t", 0) == 0)
    {
        hostBypassed.store(line.size() > 11 && line[11] == '1', std::memory_order_release);
        return "OK";
    }
    if (line.rfind("RENDER\t", 0) == 0) return offline ? Render(line) : "ERROR\tThis host plays live; start one with --offline to render.";
    return "ERROR\tUnknown command.";
}

constexpr UINT WM_RUN_COMMAND = WM_APP + 1, WM_STOP_HOST = WM_APP + 2;
struct Pending { std::string id, line, reply; HANDLE ready; };
HWND commandWindow = nullptr;

LRESULT CALLBACK CommandProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    if (msg == WM_RUN_COMMAND)
    {
        auto* p = reinterpret_cast<Pending*>(wp);
        p->reply = Run(p->line);
        SetEvent(p->ready);
        return 0;
    }
    if (msg == WM_STOP_HOST)
    {
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

// ---- Processing one block: MIDI-style events + optional stereo audio in, stereo out (live and offline) ----
struct Block
{
    HostProcessData data;
    EventList events{MaxEvents};
    ParameterChanges params{64};
    ProcessContext context{};
    bool hasBypassParamState = false;
    bool lastBypassParamValue = false;

    Block()
    {
        data.prepare(*component, MaxFrames, kSample32);
        context.sampleRate = configuredSampleRate;
        context.tempo = 120;
        context.timeSigNumerator = 4;
        context.timeSigDenominator = 4;
        context.state = ProcessContext::kPlaying | ProcessContext::kTempoValid | ProcessContext::kTimeSigValid;
    }

    static ParamID Map(int channel, int cc) { return midiMap[channel * kCountCtrlNumber + cc].load(std::memory_order_relaxed); }

    void Change(ParamID id, int offset, ParamValue value)
    {
        if (id == kNoParamId) return;
        int32 index = 0;
        if (auto* queue = params.addParameterData(id, index)) queue->addPoint(offset, value, index);
    }

    void ApplyBypass(bool bypassed)
    {
        if (bypassParamId != kNoParamId && (!hasBypassParamState || lastBypassParamValue != bypassed))
        {
            Change(bypassParamId, 0, bypassed ? 1.0 : 0.0);
            hasBypassParamState = true;
            lastBypassParamValue = bypassed;
        }
    }

    void SyncContext(const ShmProcessContext& src)
    {
        if (src.sampleRate > 0.0 && std::isfinite(src.sampleRate)) context.sampleRate = src.sampleRate;
        if (src.tempo > 0.0 && std::isfinite(src.tempo)) context.tempo = src.tempo;
        if (src.timeSigNum > 0) context.timeSigNumerator = src.timeSigNum;
        if (src.timeSigDen > 0) context.timeSigDenominator = src.timeSigDen;
        context.projectTimeSamples = src.projectTimeSamples;
        context.projectTimeMusic = src.projectTimeMusic;
        context.barPositionMusic = src.barPositionMusic;
        context.cycleStartMusic = src.cycleStartMusic;
        context.cycleEndMusic = src.cycleEndMusic;
        if (src.stateFlags != 0) context.state = src.stateFlags;
    }

    void Begin()
    {
        events.clear();
        params.clearQueue();
        std::lock_guard<std::mutex> g(handler.lock);
        for (auto& [id, value] : handler.pending) Change(id, 0, value);
        handler.pending.clear();
    }

    void Add(int offset, uint8_t status, uint8_t data1, uint8_t data2)
    {
        data1 &= 0x7F;
        data2 &= 0x7F;
        int kind = status & 0xF0, channel = status & 0x0F;
        Event ev{};
        ev.busIndex = 0;
        ev.sampleOffset = offset;
        if (kind == 0x90 && data2 > 0)
        {
            ev.type = Event::kNoteOnEvent;
            ev.noteOn.channel = (int16)channel;
            ev.noteOn.pitch = (int16)data1;
            ev.noteOn.velocity = data2 / 127.f;
            ev.noteOn.noteId = -1;
            events.addEvent(ev);
        }
        else if (kind == 0x80 || kind == 0x90)
        {
            ev.type = Event::kNoteOffEvent;
            ev.noteOff.channel = (int16)channel;
            ev.noteOff.pitch = (int16)data1;
            ev.noteOff.noteId = -1;
            events.addEvent(ev);
        }
        else if (kind == 0xB0) Change(Map(channel, data1), offset, data2 / 127.0);
        else if (kind == 0xE0) Change(Map(channel, kPitchBend), offset, (data2 << 7 | data1) / 16383.0);
    }

    static bool IsBufferSilent(const float* buf, int frames)
    {
        for (int i = 0; i < frames; i++)
            if (buf[i] != 0.0f) return false;
        return true;
    }

    bool Process(int frames, const float* inLeft, const float* inRight, float* left, float* right, bool hardBypassFallback)
    {
        if (hardBypassFallback)
        {
            if (inLeft) memcpy(left, inLeft, frames * sizeof(float));
            else memset(left, 0, frames * sizeof(float));
            if (inRight) memcpy(right, inRight, frames * sizeof(float));
            else if (inLeft) memcpy(right, inLeft, frames * sizeof(float));
            else memset(right, 0, frames * sizeof(float));
            context.projectTimeSamples += frames;
            return true;
        }

        std::lock_guard<std::mutex> g(processLock);
        data.numSamples = frames;
        for (int32 b = 0; b < data.numInputs; b++)
        {
            auto& bus = data.inputs[b];
            uint64 silence = 0;
            for (int32 c = 0; c < bus.numChannels; c++)
            {
                float* dst = bus.channelBuffers32 ? bus.channelBuffers32[c] : nullptr;
                if (!dst)
                {
                    if (c < 64) silence |= (1ULL << c);
                    continue;
                }
                if (b == 0 && c == 0 && inLeft)
                    memcpy(dst, inLeft, frames * sizeof(float));
                else if (b == 0 && c == 1 && (inRight || inLeft))
                    memcpy(dst, inRight ? inRight : inLeft, frames * sizeof(float));
                else
                    memset(dst, 0, frames * sizeof(float));

                if (c < 64 && IsBufferSilent(dst, frames)) silence |= (1ULL << c);
            }
            bus.silenceFlags = silence;
        }
        for (int32 b = 0; b < data.numOutputs; b++)
        {
            data.outputs[b].silenceFlags = 0;
        }
        data.inputEvents = &events;
        data.inputParameterChanges = &params;
        data.processContext = &context;
        bool ok = processor->process(data) == kResultOk && data.numOutputs > 0 && data.outputs[0].numChannels > 0;
        if (ok)
        {
            const float* l = data.outputs[0].channelBuffers32[0];
            const float* r = data.outputs[0].numChannels > 1 ? data.outputs[0].channelBuffers32[1] : l;
            for (int i = 0; i < frames && ok; i++) ok = std::isfinite(l[i]) && std::isfinite(r[i]);
            if (ok)
            {
                memcpy(left, l, frames * sizeof(float));
                memcpy(right, r, frames * sizeof(float));
            }
        }
        if (!ok)
        {
            memset(left, 0, frames * sizeof(float));
            memset(right, 0, frames * sizeof(float));
        }
        context.projectTimeSamples += frames;
        return ok;
    }
};

// ---- Offline rendering of one note (Generate) ----
std::string Render(const std::string& line)
{
    std::vector<std::string> f;
    for (size_t at = 0;;)
    {
        size_t tab = line.find('\t', at);
        f.push_back(line.substr(at, tab == std::string::npos ? std::string::npos : tab - at));
        if (tab == std::string::npos) break;
        at = tab + 1;
    }
    if (f.size() != 6 || f[0] != "RENDER") return "ERROR\tusage: RENDER <file> <max> <quiet> <threshold> <events>";
    long long maxFrames = _atoi64(f[2].c_str()), quietFrames = _atoi64(f[3].c_str());
    double threshold = atof(f[4].c_str());
    long long sr = std::max<long long>(8000, (long long)std::llround(configuredSampleRate));
    if (maxFrames < 1 || maxFrames > sr * 600 || quietFrames < 1 || !(threshold > 0)) return "ERROR\tBad render limits.";

    struct Ev { long long frame; uint8_t s, d1, d2; };
    std::vector<Ev> evs;
    for (size_t at = 0; at < f[5].size();)
    {
        size_t end = f[5].find(';', at);
        if (end == std::string::npos) end = f[5].size();
        long long frame; unsigned st, d1, d2;
        if (sscanf_s(f[5].substr(at, end - at).c_str(), "%lld:%u:%u:%u", &frame, &st, &d1, &d2) == 4 && frame >= 0 && st < 256 && d1 < 128 && d2 < 128)
            evs.push_back({frame, (uint8_t)st, (uint8_t)d1, (uint8_t)d2});
        at = end + 1;
    }
    std::stable_sort(evs.begin(), evs.end(), [](const Ev& a, const Ev& b) { return a.frame < b.frame; });
    long long lastEvent = evs.empty() ? 0 : evs.back().frame;

    constexpr int Chunk = 512;
    static Block block;
    std::vector<float> l(Chunk), r(Chunk);
    auto Peak = [&](int n) { float p = 0; for (int i = 0; i < n; i++) p = std::max(p, std::max(std::fabs(l[i]), std::fabs(r[i]))); return p; };

    // 16-channel full MIDI reset processed in 4-channel batches so `events` (MaxEvents = 1024) never overflows.
    auto FlushAll16Channels = [&] {
        for (int chBase = 0; chBase < 16; chBase += 4)
        {
            block.Begin();
            for (int ch = chBase; ch < chBase + 4; ch++)
            {
                for (int pitch = 0; pitch < 128; pitch++) block.Add(0, (uint8_t)(0x80 | ch), (uint8_t)pitch, 0);
                block.Add(0, (uint8_t)(0xB0 | ch), 64, 0);
                block.Add(0, (uint8_t)(0xB0 | ch), 1, 0);
                block.Add(0, (uint8_t)(0xB0 | ch), 120, 0);
                block.Add(0, (uint8_t)(0xB0 | ch), 123, 0);
                block.Add(0, (uint8_t)(0xE0 | ch), 0, 64);
            }
            block.Process(64, nullptr, nullptr, l.data(), r.data(), false);
        }
        block.Begin();
    };

    static float idleFloor = -1;
    if (idleFloor < 0)
    {
        FlushAll16Channels();
        idleFloor = 0;
        for (long long done = 0; done < sr; done += Chunk)
        {
            block.Process(Chunk, nullptr, nullptr, l.data(), r.data(), false);
            if (done >= sr / 2) idleFloor = std::max(idleFloor, Peak(Chunk));
            block.Begin();
        }
    }
    threshold = std::max(threshold, (double)idleFloor * 2);

    FlushAll16Channels();
    for (long long done = 0, quiet = 0; done < sr * 5 && quiet < quietFrames; done += Chunk)
    {
        block.Process(Chunk, nullptr, nullptr, l.data(), r.data(), false);
        quiet = Peak(Chunk) < threshold ? quiet + Chunk : 0;
        block.Begin();
    }

    std::vector<float> out;
    size_t next = 0;
    long long lastLoud = -1, quiet = 0, frame = 0;
    float peak = 0;
    bool truncated = true;
    while (frame < maxFrames)
    {
        int n = (int)std::min<long long>(Chunk, maxFrames - frame);
        block.Begin();
        while (next < evs.size() && evs[next].frame < frame + n)
        {
            block.Add((int)(evs[next].frame - frame), evs[next].s, evs[next].d1, evs[next].d2);
            next++;
        }
        if (!block.Process(n, nullptr, nullptr, l.data(), r.data(), false)) return "ERROR\tThe plug-in failed while rendering.";
        for (int i = 0; i < n; i++)
        {
            out.push_back(l[i]);
            out.push_back(r[i]);
            float a = std::max(std::fabs(l[i]), std::fabs(r[i]));
            peak = std::max(peak, a);
            if (a >= threshold) lastLoud = frame + i;
        }
        frame += n;
        quiet = frame - 1 - std::max(lastLoud, lastEvent);
        if (frame > lastEvent && quiet >= quietFrames) { truncated = false; break; }
    }

    long long keep = truncated ? frame : std::max(lastLoud, lastEvent) + 1;
    FILE* file = nullptr;
    std::wstring path(MultiByteToWideChar(CP_UTF8, 0, f[1].data(), (int)f[1].size(), nullptr, 0), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, f[1].data(), (int)f[1].size(), path.data(), (int)path.size());
    if (_wfopen_s(&file, path.c_str(), L"wb") != 0 || !file) return "ERROR\tThe render file couldn't be written.";
    size_t wrote = fwrite(out.data(), sizeof(float), (size_t)keep * 2, file);
    fclose(file);
    if (wrote != (size_t)keep * 2) return "ERROR\tThe render file couldn't be written.";
    char reply[96];
    snprintf(reply, sizeof reply, "RENDERED\t%lld\t%d\t%.6f", keep, truncated ? 1 : 0, peak);
    return reply;
}

// ---- Audio: one block per request (MMCSS Pro Audio) ----
std::atomic<bool> quitting{false};

void AudioThread(Shm* shm, HANDLE req, HANDLE done, HANDLE parent)
{
    DWORD taskIndex = 0;
    HANDLE mmcss = AvSetMmThreadCharacteristicsW(L"Pro Audio", &taskIndex);
    if (!mmcss) SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);

    Block block;
    HANDLE waits[2] = {req, parent};
    while (!quitting)
    {
        DWORD w = WaitForMultipleObjects(2, waits, FALSE, 500);
        if (w == WAIT_OBJECT_0 + 1) ExitProcess(0);
        if (w != WAIT_OBJECT_0) continue;

        uint32_t reqSeq = shm->requestSeq.load(std::memory_order_acquire);
        if (pendingLatencyReactivate.exchange(false, std::memory_order_acq_rel))
        {
            std::lock_guard<std::mutex> g(processLock);
            processor->setProcessing(false);
            component->setActive(false);
            component->setActive(true);
            processor->setProcessing(true);
            RefreshLatencyAndTail();
        }

        int frames = shm->frames < 1 ? 0 : shm->frames > MaxFrames ? MaxFrames : shm->frames;
        int count = shm->eventCount < 0 ? 0 : shm->eventCount > MaxEvents ? MaxEvents : shm->eventCount;
        double requestedRate = (shm->context.sampleRate > 0 && std::isfinite(shm->context.sampleRate))
            ? shm->context.sampleRate
            : configuredSampleRate;
        if (std::fabs(requestedRate - configuredSampleRate) > 0.5)
        {
            std::lock_guard<std::mutex> g(processLock);
            processor->setProcessing(false);
            component->setActive(false);
            std::string reconfigErr;
            if (!ConfigureProcessing(requestedRate, reconfigErr))
            {
                ConfigureProcessing(configuredSampleRate, reconfigErr);
            }
        }

        uint32_t shmFlags = shm->flags.load(std::memory_order_relaxed);
        bool hasAudioIn = (shmFlags & ShmFlagHasAudioIn) != 0;
        bool bypassed = hostBypassed.load(std::memory_order_acquire) || ((shmFlags & ShmFlagBypassed) != 0);

        block.SyncContext(shm->context);
        block.context.sampleRate = configuredSampleRate;
        block.Begin();
        block.ApplyBypass(bypassed);
        for (int i = 0; i < count; i++)
        {
            auto& e = shm->events[i];
            int offset = e.offset < 0 ? 0 : e.offset >= frames ? (frames > 0 ? frames - 1 : 0) : e.offset;
            block.Add(offset, e.status, e.data1, e.data2);
        }

        const float* inL = hasAudioIn ? shm->inLeft : nullptr;
        const float* inR = hasAudioIn ? shm->inRight : nullptr;
        bool hardBypass = bypassed && (bypassParamId == kNoParamId);
        uint32_t status = 1;
        if (frames > 0 && block.Process(frames, inL, inR, shm->left, shm->right, hardBypass)) status = 0;
        RefreshLatencyAndTail();
        shm->status.store(status, std::memory_order_release);
        shm->doneSeq.store(reqSeq, std::memory_order_release);
        SetEvent(done);
    }

    if (mmcss) AvRevertMmThreadCharacteristics(mmcss);
}

} // namespace

int wmain(int argc, wchar_t** argv)
{
    auto Arg = [&](int i) {
        int n = WideCharToMultiByte(CP_UTF8, 0, argv[i], -1, nullptr, 0, nullptr, nullptr);
        std::string s(n > 0 ? n - 1 : 0, '\0');
        WideCharToMultiByte(CP_UTF8, 0, argv[i], -1, s.data(), n, nullptr, nullptr);
        return s;
    };
    IsolateStdout();
    if (argc == 3 && Arg(1) == "--scan") return Scan(Arg(2));
    if (argc < 5 || Arg(1) != "--host") { Reply("ERROR\tusage: --scan <plugin.vst3> | --host <name> <parentPid> <plugin.vst3> [<uid>] [--offline]"); return 1; }
    std::string uid;
    double requestedSampleRate = 0.0;
    for (int i = 5; i < argc; i++)
    {
        std::string a = Arg(i);
        if (a == "--offline") offline = true;
        else if (a == "--sample-rate" && i + 1 < argc) requestedSampleRate = atof(Arg(++i).c_str());
        else uid = a;
    }

    std::wstring name = argv[2];
    DWORD parentPid = (DWORD)_wtoi(argv[3]);
    HANDLE parent = OpenProcess(SYNCHRONIZE, FALSE, parentPid);
    if (!parent) { Reply("ERROR\tdawsu! is not running."); return 1; }

    HANDLE mapping = OpenFileMappingW(FILE_MAP_ALL_ACCESS, FALSE, (L"Local\\" + name).c_str());
    HANDLE req = OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, FALSE, (L"Local\\" + name + L"-req").c_str());
    HANDLE done = OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, FALSE, (L"Local\\" + name + L"-done").c_str());
    auto* shm = mapping ? (Shm*)MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, sizeof(Shm)) : nullptr;
    if (!shm || !req || !done || shm->magic != Magic) { Reply("ERROR\tThe shared audio buffer is missing."); return 1; }
    if (shm->version != Protocol) { Reply("ERROR\tdawsu_vst3_host.exe doesn't match this version of dawsu!; reinstall dawsu!."); return 1; }
    globalShm = shm;

    OleInitialize(nullptr);
    PluginContextFactory::instance().setPluginContext(&hostApp);
    WNDCLASSW wc{};
    wc.lpfnWndProc = EditorProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"DawsuVst3Editor";
    wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
    RegisterClassW(&wc);

    WNDCLASSW commands{};
    commands.lpfnWndProc = CommandProc;
    commands.hInstance = wc.hInstance;
    commands.lpszClassName = L"DawsuVst3Commands";
    RegisterClassW(&commands);
    commandWindow = CreateWindowExW(0, commands.lpszClassName, nullptr, 0, 0, 0, 0, 0, HWND_MESSAGE, nullptr, wc.hInstance, nullptr);
    if (!commandWindow) { Reply("ERROR\tThe VST3 host couldn't start its command window."); return 1; }

    double initialSampleRate = requestedSampleRate > 0.0
        ? requestedSampleRate
        : (shm->context.sampleRate > 0.0 && std::isfinite(shm->context.sampleRate) ? shm->context.sampleRate : 44100.0);
    std::string error;
    if (!Load(Arg(4), uid, initialSampleRate, error)) { Reply("ERROR\t" + Clean(error)); return 2; }
    Reply("READY\t" + Clean(pluginName) + "\t" + std::to_string(Protocol));

    std::thread audio(AudioThread, shm, req, done, parent);
    std::thread reader([] {
        std::string line;
        while (std::getline(std::cin, line))
        {
            if (!line.empty() && line.back() == '\r') line.pop_back();
            size_t tab = line.find('\t');
            if (tab == std::string::npos) continue;
            std::string id = line.substr(0, tab), command = line.substr(tab + 1);
            if (command == "QUIT") break;
            Pending p{id, command, "", CreateEventW(nullptr, TRUE, FALSE, nullptr)};
            if (!p.ready || !PostMessageW(commandWindow, WM_RUN_COMMAND, reinterpret_cast<WPARAM>(&p), 0))
                p.reply = "ERROR\tThe VST3 host couldn't run the command.";
            else
                WaitForSingleObject(p.ready, INFINITE);
            if (p.ready) CloseHandle(p.ready);
            Reply(p.id + "\t" + p.reply);
        }
        PostMessageW(commandWindow, WM_STOP_HOST, 0, 0);
    });

    MSG msg;
    while (GetMessageW(&msg, nullptr, 0, 0) > 0)
    {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    quitting = true;
    audio.join();
    reader.detach();
    globalShm = nullptr;
    if (view) { view->removed(); view = nullptr; }
    if (processor) processor->setProcessing(false);
    if (component) component->setActive(false);
    midiMapping = nullptr;
    controller = nullptr;
    processor = nullptr;
    component = nullptr;
    provider = nullptr;
    module = nullptr;
    PluginContextFactory::instance().setPluginContext(nullptr);
    return 0;
}

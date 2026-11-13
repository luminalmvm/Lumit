#include "notify.h"

#include <winrt/Windows.Data.Xml.Dom.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Notifications.h>

#pragma comment(lib, "windowsapp")

namespace {

using winrt::Windows::Data::Xml::Dom::XmlDocument;
using winrt::Windows::Foundation::IInspectable;
using winrt::Windows::UI::Notifications::ToastNotification;
using winrt::Windows::UI::Notifications::ToastNotificationManager;

// The name the system keeps Lumit's notifications under, the same as Linux's
// application id. Lumit isn't a packaged app, so it says in the registry what
// that name is called and which icon it wears. The process keeps the id
// Windows gives it, which is what the Start menu's shortcut and a pinned
// taskbar button have too.
constexpr wchar_t kAppId[] = L"io.github.luminalmvm.Lumit";

// Write Lumit's name and icon under kAppId, unless they are there already for
// this copy of Lumit. The icon is the one installed beside the executable
// (windows/CMakeLists.txt).
void Register() {
  std::wstring icon(32768, L'\0');
  icon.resize(::GetModuleFileNameW(nullptr, icon.data(),
                                   static_cast<DWORD>(icon.size())));
  icon.resize(icon.find_last_of(L'\\') + 1);
  icon += L"data\\app_icon.ico";
  const std::wstring key =
      std::wstring(L"Software\\Classes\\AppUserModelId\\") + kAppId;
  std::wstring kept(32768, L'\0');
  DWORD size = static_cast<DWORD>(kept.size() * sizeof(wchar_t));
  if (::RegGetValueW(HKEY_CURRENT_USER, key.c_str(), L"IconUri",
                     RRF_RT_REG_SZ, nullptr, kept.data(),
                     &size) == ERROR_SUCCESS &&
      icon == kept.c_str()) {
    return;
  }
  HKEY handle = nullptr;
  if (::RegCreateKeyExW(HKEY_CURRENT_USER, key.c_str(), 0, nullptr, 0,
                        KEY_SET_VALUE, nullptr, &handle,
                        nullptr) != ERROR_SUCCESS) {
    return;
  }
  const wchar_t name[] = L"Lumit";
  ::RegSetValueExW(handle, L"DisplayName", 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(name), sizeof(name));
  ::RegSetValueExW(handle, L"IconUri", 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(icon.c_str()),
                   static_cast<DWORD>((icon.size() + 1) * sizeof(wchar_t)));
  ::RegCloseKey(handle);
}

// [text] as XML text, its special characters written as entities.
std::wstring Escaped(const std::wstring& text) {
  std::wstring out;
  for (const wchar_t c : text) {
    switch (c) {
      case L'&':
        out += L"&amp;";
        break;
      case L'<':
        out += L"&lt;";
        break;
      case L'>':
        out += L"&gt;";
        break;
      case L'"':
        out += L"&quot;";
        break;
      default:
        out += c;
    }
  }
  return out;
}

}  // namespace

void ShowNotification(HWND window, const std::wstring& title,
                      const std::wstring& body) {
  // The last one shown, held so its click still reaches this process. Never
  // freed: a WinRT object let go after COM has closed at exit can crash.
  struct Held {
    ToastNotification toast{nullptr};
  };
  static Held* last = new Held();
  try {
    Register();
    std::wstring text = L"<text>" + Escaped(title) + L"</text>";
    if (!body.empty()) {
      text += L"<text>" + Escaped(body) + L"</text>";
    }
    XmlDocument xml;
    xml.LoadXml(L"<toast><visual><binding template=\"ToastGeneric\">" + text +
                L"</binding></visual></toast>");
    ToastNotification toast(xml);
    toast.Activated([window](const ToastNotification&, const IInspectable&) {
      ::PostMessage(window, kNotifyClickMessage, 0, 0);
    });
    ToastNotificationManager::CreateToastNotifier(kAppId).Show(toast);
    last->toast = toast;
  } catch (...) {
    // Notifications turned off, or a Windows without them.
  }
}

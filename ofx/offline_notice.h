#pragma once
#include <windows.h>
#include <vector>
#include <stdexcept>

// A fixed diagnostic image, created only if the renderer is unreachable.
struct OfflineNotice {
    static constexpr unsigned width = 1280, height = 720;
    std::vector<unsigned char> pixels;
    OfflineNotice() : pixels(size_t(width) * height * 4) {
        HDC dc = CreateCompatibleDC(nullptr);
        BITMAPINFO info{}; info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        info.bmiHeader.biWidth = width; info.bmiHeader.biHeight = -int(height);
        info.bmiHeader.biPlanes = 1; info.bmiHeader.biBitCount = 32; info.bmiHeader.biCompression = BI_RGB;
        void* data{}; HBITMAP bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &data, nullptr, 0);
        if (!dc || !bitmap) { if (bitmap) DeleteObject(bitmap); if (dc) DeleteDC(dc); throw std::runtime_error("Cannot draw the offline notice."); }
        auto previous = SelectObject(dc, bitmap);
        memset(data, 0, pixels.size());
        HBRUSH background = CreateSolidBrush(RGB(27, 29, 34));
        RECT panel{320, 272, 960, 448}; FillRect(dc, &panel, background);
        DeleteObject(background);
        SetBkMode(dc, TRANSPARENT);
        HFONT title = CreateFontW(-30, 0, 0, 0, FW_SEMIBOLD, FALSE, FALSE, FALSE,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, DEFAULT_PITCH, L"Segoe UI");
        HFONT body = CreateFontW(-22, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, DEFAULT_PITCH, L"Segoe UI");
        auto previousFont = SelectObject(dc, title);
        SetTextColor(dc, RGB(240, 241, 244));
        RECT heading{500, 296, 806, 338}; DrawTextW(dc, L"Lumit is offline", -1, &heading, DT_LEFT | DT_SINGLELINE);
        HBRUSH amber = CreateSolidBrush(RGB(240, 183, 66));
        auto previousBrush = SelectObject(dc, amber); auto previousPen = SelectObject(dc, GetStockObject(NULL_PEN));
        POINT triangle[]{{474, 300}, {458, 328}, {490, 328}}; Polygon(dc, triangle, 3);
        SelectObject(dc, previousBrush); SelectObject(dc, previousPen); DeleteObject(amber);
        SetTextColor(dc, RGB(27, 29, 34)); SelectObject(dc, body);
        RECT mark{458, 300, 490, 331}; DrawTextW(dc, L"!", -1, &mark, DT_CENTER | DT_SINGLELINE);
        SetTextColor(dc, RGB(208, 210, 217));
        RECT line{330, 352, 950, 384}; DrawTextW(dc, L"Click \x201cOpen in Lumit\x201d in the effect controls", -1, &line, DT_CENTER | DT_SINGLELINE);
        line.top = 386; line.bottom = 418; DrawTextW(dc, L"to render this composition.", -1, &line, DT_CENTER | DT_SINGLELINE);
        GdiFlush();
        const auto source = static_cast<const unsigned char*>(data);
        for (int y = panel.top; y < panel.bottom; ++y) for (int x = panel.left; x < panel.right; ++x) {
            auto at = (size_t(y) * width + x) * 4;
            pixels[at] = source[at + 2]; pixels[at + 1] = source[at + 1]; pixels[at + 2] = source[at]; pixels[at + 3] = 255;
        }
        SelectObject(dc, previousFont); SelectObject(dc, previous);
        DeleteObject(title); DeleteObject(body); DeleteObject(bitmap); DeleteDC(dc);
    }
};

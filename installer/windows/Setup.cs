using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Drawing.Text;
using System.IO;
using System.IO.Compression;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;
using Microsoft.Win32;

// Pixel Music setup and uninstaller. One window drawn entirely by this file (no stock WinForms controls except
// the folder text box), in the colours of the Pixel Music logo. The approach comes from SoundWave's installer:
// built with the .NET Framework C# 5 compiler that ships with Windows (build_windows.ps1), so no newer language
// features here. The same file builds both programs:
//   Setup:      csc /resource: payload.zip, Uninstall.exe, logo.png, version.txt
//   Uninstall:  csc /define:UNINSTALLER /resource: logo.png, version.txt
//
// Per-user install into %LOCALAPPDATA%\Programs\Pixel Music, no administrator rights. It installs over an Inno Setup
// install of Pixel Music (same folder, same "Installed apps" entry) and understands the flags the in-app updater
// passes to the setup: /SILENT (progress window only), /VERYSILENT (no window), /CLOSEAPPLICATIONS, /RELAUNCH,
// /DIR="folder". The rest of Inno's flags (/SP-, /NOCANCEL, /NORESTART, /SUPPRESSMSGBOXES …) are accepted and ignored.
//
// Development: `<exe> --screenshot <page 0|1|2> <file.png>` saves a picture of one page and exits, installing
// (or removing) nothing.
namespace PixelMusic.Installer
{
    public class SetupForm : Form
    {
        [DllImport("user32.dll")]
        private static extern bool ReleaseCapture();
        [DllImport("user32.dll")]
        private static extern int SendMessage(IntPtr hWnd, int msg, int wParam, int lParam);
        [DllImport("user32.dll")]
        private static extern bool SetProcessDPIAware();
        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);

        // ── Names ─────────────────────────────────────────────────────────────────────────
        private const string AppName = "Pixel Music";
        // the name Windows shows: the installer's screens, the shortcuts (the taskbar jump list and the media flyout
        // take the Start-menu shortcut's name), Installed apps. The install folder keeps AppName so updates find it.
        private const string ShownName = "PixelMusic";
        private const string ExeName = "pixelmusic.exe";
        private const string Publisher = "Ian Shul Yadav";
        // the Inno Setup AppId of pixelmusic.iss: one "Installed apps" entry whichever installer was used
        private const string UninstallKey = @"Software\Microsoft\Windows\CurrentVersion\Uninstall\{A5260B19-41AE-4D9D-9841-204E686F2ADD}_is1";
        // the taskbar / notification identity the app sets for itself (AppUserModelID on its shortcuts)
        private const string AppUserModelId = "Pixel Music";

        // ── Colours: the Pixel Music logo's blues on a dark navy ──────────────────────────
        private static readonly Color Bg = Color.FromArgb(12, 16, 27);              // #0C101B
        private static readonly Color Surface = Color.FromArgb(25, 32, 50);         // #192032
        private static readonly Color SurfaceHover = Color.FromArgb(36, 46, 70);    // #242E46
        private static readonly Color Outline = Color.FromArgb(48, 60, 88);         // #303C58
        private static readonly Color Text1 = Color.FromArgb(232, 238, 251);        // #E8EEFB
        private static readonly Color Text2 = Color.FromArgb(172, 184, 210);        // #ACB8D2
        private static readonly Color Text3 = Color.FromArgb(116, 128, 156);        // #74809C
        private static readonly Color Accent = Color.FromArgb(160, 202, 255);       // #A0CAFF
        private static readonly Color AccentHover = Color.FromArgb(186, 218, 255);  // #BADAFF
        private static readonly Color AccentDeep = Color.FromArgb(52, 152, 254);    // #3498FE (logo)
        private static readonly Color OnAccent = Color.FromArgb(0, 44, 96);         // #002C60
        private static readonly Color Danger = Color.FromArgb(255, 180, 171);       // #FFB4AB
        private static readonly Color PaneTop = Color.FromArgb(24, 88, 214);        // #1858D6
        private static readonly Color PaneBottom = Color.FromArgb(14, 20, 58);      // #0E143A
        private static readonly Color Glow = Color.FromArgb(120, 186, 255);         // #78BAFF
        private static readonly Color GlowPurple = Color.FromArgb(180, 150, 254);   // #B496FE (logo)

        private const int PageOptions = 0, PageWorking = 1, PageDone = 2;

        /** A clickable area drawn by OnPaint. */
        private class HotSpot
        {
            public string Id;
            public Rectangle Bounds;
            public Action Click;
        }

        private readonly float k;                       // pixels per 96-dpi unit
        private readonly List<HotSpot> spots = new List<HotSpot>();
        private string hoverId;
        private string pressedId;

        private int page = PageOptions;
        private bool optDesktop = true, optStartMenu = true, optAutostart, optLaunch = true, optKeepData = true;
        private int progress;
        private string progressDetail = "";
        private string errorText;
        private bool updating;                          // the chosen folder already has the app: "Update", else "Install"
        private bool closeFirst;                        // Pixel Music is running: Install closes it
        private readonly string version;
        private readonly long payloadBytes;
        private string installedDir = "";

        // command line
        private bool silent;
#pragma warning disable 414 // relaunch is only read by Setup, not by the uninstaller build
        private readonly bool verySilent, relaunch;
#pragma warning restore 414
        private bool cleanedUp;

        private Image logo;
        private TextBox txtPath;
        private bool screenshotting;
        private readonly System.Windows.Forms.Timer waveTimer = new System.Windows.Forms.Timer();
        private double wavePhase;
        private double shownProgress;                   // the bar glides to `progress`

        private readonly Font fTitle, fBody, fBodyBold, fSmall, fLabel, fHuge, fBrand;

        protected override CreateParams CreateParams
        {
            get
            {
                CreateParams cp = base.CreateParams;
                cp.ClassStyle |= 0x00020000; // CS_DROPSHADOW
                return cp;
            }
        }

        public SetupForm(string[] args)
        {
            using (Graphics g = CreateGraphics()) { k = g.DpiX / 96f; }

            // the app's own typeface (Google Sans, three fixed weights embedded in this exe); Segoe UI if they don't load
            LoadFonts();
            fBrand = AppFont("Google Sans", 20f, FontStyle.Bold, "Segoe UI", FontStyle.Bold);
            fTitle = AppFont("Google Sans Medium", 16f, FontStyle.Regular, "Segoe UI Semibold", FontStyle.Regular);
            fBody = AppFont("Google Sans", 9.5f, FontStyle.Regular, "Segoe UI", FontStyle.Regular);
            fBodyBold = AppFont("Google Sans Medium", 10f, FontStyle.Regular, "Segoe UI Semibold", FontStyle.Regular);
            fSmall = AppFont("Google Sans", 8.5f, FontStyle.Regular, "Segoe UI", FontStyle.Regular);
            fLabel = AppFont("Google Sans Medium", 7.5f, FontStyle.Regular, "Segoe UI Semibold", FontStyle.Regular);
            fHuge = AppFont("Google Sans", 40f, FontStyle.Regular, "Segoe UI Light", FontStyle.Regular);

            string dirArg = null;
            foreach (string a in args)
            {
                string u = a.ToUpperInvariant();
                if (u == "/SILENT") silent = true;
                else if (u == "/VERYSILENT") { silent = true; verySilent = true; }
                else if (u == "/RELAUNCH") relaunch = true;
                else if (u.StartsWith("/DIR=")) dirArg = a.Substring(5).Trim('"');
            }

            version = ReadVersion();
#if UNINSTALLER
            payloadBytes = 0;
#else
            payloadBytes = MeasurePayload();
#endif

#if UNINSTALLER
            Text = "Uninstall " + ShownName;
#else
            Text = ShownName + " Setup";
#endif
            FormBorderStyle = FormBorderStyle.None;
            StartPosition = FormStartPosition.CenterScreen;
            ClientSize = new Size(S(820), S(520));
            BackColor = Bg;
            KeyPreview = true;
            SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
            try { Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath); } catch { }

            string dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"Programs\" + AppName);
            string existing = ExistingInstall();
            if (existing != null) dir = existing;
            // autostart is off by default (as in pixelmusic.iss); an update keeps what the user had
            optAutostart = File.Exists(StartupLink()) || File.Exists(LegacyStartupLink());
#if UNINSTALLER
            // the uninstaller removes the folder it sits in
            dir = Path.GetDirectoryName(Application.ExecutablePath);
#endif
            if (!string.IsNullOrEmpty(dirArg)) dir = dirArg;

            LoadLogo();

            txtPath = new TextBox();
            txtPath.BorderStyle = BorderStyle.None;
            txtPath.BackColor = Surface;
            txtPath.ForeColor = Text1;
            txtPath.Font = fBody;
            txtPath.Text = dir;
            updating = HasApp(dir);
            // follows the folder as it is typed or browsed: another folder without the app is a fresh install
            txtPath.TextChanged += delegate { errorText = null; closeFirst = false; updating = HasApp(txtPath.Text); Invalidate(); };
            Controls.Add(txtPath);

            waveTimer.Interval = 40;
            waveTimer.Tick += delegate
            {
                wavePhase += 0.18;
                Invalidate(WaveArea());
                if (Math.Abs(shownProgress - progress) > 0.05)
                {
                    shownProgress += (progress - shownProgress) * 0.25;
                    Invalidate(ProgressArea());
                }
            };
            waveTimer.Start();

            Relayout();
        }

        protected override void OnShown(EventArgs e)
        {
            base.OnShown(e);
            // Don't open with the folder path selected and highlighted.
            ActiveControl = null;
            txtPath.SelectionLength = 0;
            if (silent) StartWork();
        }

        protected override void OnHandleCreated(EventArgs e)
        {
            base.OnHandleCreated(e);
            // Rounded window corners on Windows 11; older Windows keeps square corners.
            try { int round = 2; DwmSetWindowAttribute(Handle, 33, ref round, sizeof(int)); } catch { }
            // Dark title-bar colours for the system parts (Alt+Tab preview, snap).
            try { int dark = 1; DwmSetWindowAttribute(Handle, 20, ref dark, sizeof(int)); } catch { }
        }

        // ── Helpers ───────────────────────────────────────────────────────────────────────

        private int S(double v) { return (int)Math.Round(v * k); }
        private Rectangle R(double x, double y, double w, double h) { return new Rectangle(S(x), S(y), S(w), S(h)); }

        private const int PaneWidth = 300;   // left brand pane
        private const int ContentX = 344;    // right content column
        private const int ContentW = 432;

        private static bool HasApp(string dir)
        {
            try { return File.Exists(Path.Combine(dir.Trim(), ExeName)); } catch { return false; }
        }

        [DllImport("gdi32.dll")]
        private static extern IntPtr AddFontMemResourceEx(IntPtr pbFont, uint cbFont, IntPtr pdv, out uint pcFonts);

        // GDI+ needs the families from this collection to make Font objects, and GDI (TextRenderer draws the text)
        // finds the same faces by name through AddFontMemResourceEx; the memory stays alive for the whole run.
        private static readonly PrivateFontCollection appFonts = new PrivateFontCollection();
        private static bool fontsLoaded;

        private static void LoadFonts()
        {
            if (fontsLoaded) return;
            fontsLoaded = true;
            foreach (string name in new string[] { "GoogleSans-Regular.ttf", "GoogleSans-Medium.ttf", "GoogleSans-Bold.ttf" })
            {
                try
                {
                    using (Stream s = Assembly.GetExecutingAssembly().GetManifestResourceStream(name))
                    {
                        if (s == null) continue;
                        byte[] data = new byte[s.Length];
                        int read = 0;
                        while (read < data.Length)
                        {
                            int n = s.Read(data, read, data.Length - read);
                            if (n <= 0) break;
                            read += n;
                        }
                        IntPtr mem = Marshal.AllocCoTaskMem(data.Length);
                        Marshal.Copy(data, 0, mem, data.Length);
                        uint count;
                        AddFontMemResourceEx(mem, (uint)data.Length, IntPtr.Zero, out count);
                        appFonts.AddMemoryFont(mem, data.Length);
                    }
                }
                catch { }
            }
        }

        private static Font AppFont(string family, float size, FontStyle style, string fallback, FontStyle fallbackStyle)
        {
            foreach (FontFamily f in appFonts.Families)
            {
                if (f.Name == family && f.IsStyleAvailable(style))
                {
                    try { return new Font(f, size, style); } catch { }
                }
            }
            return new Font(fallback, size, fallbackStyle);
        }

        private static string ExistingInstall()
        {
            try
            {
                using (RegistryKey key = Registry.CurrentUser.OpenSubKey(UninstallKey))
                {
                    string existing = key == null ? null : key.GetValue("InstallLocation") as string;
                    if (!string.IsNullOrEmpty(existing))
                    {
                        existing = existing.TrimEnd('\\');
                        if (File.Exists(Path.Combine(existing, ExeName))) return existing;
                    }
                }
            }
            catch { }
            return null;
        }

        private static string ReadVersion()
        {
            try
            {
                using (Stream s = Assembly.GetExecutingAssembly().GetManifestResourceStream("version.txt"))
                {
                    if (s != null) using (StreamReader r = new StreamReader(s)) { string v = r.ReadToEnd().Trim(); if (v.Length > 0) return v; }
                }
            }
            catch { }
            return "1.0.0";
        }

        /** Size of the app once unpacked (read from the zip's directory, nothing is extracted). */
        private static long MeasurePayload()
        {
            try
            {
                using (Stream s = Assembly.GetExecutingAssembly().GetManifestResourceStream("payload.zip"))
                {
                    if (s == null) return 0;
                    using (ZipArchive zip = new ZipArchive(s, ZipArchiveMode.Read))
                    {
                        long total = 0;
                        foreach (ZipArchiveEntry entry in zip.Entries) total += entry.Length;
                        return total;
                    }
                }
            }
            catch { return 0; }
        }

        private void LoadLogo()
        {
            try
            {
                using (Stream s = Assembly.GetExecutingAssembly().GetManifestResourceStream("logo.png"))
                {
                    if (s != null) logo = new Bitmap(Image.FromStream(s));
                }
            }
            catch { }
        }

        private static GraphicsPath Rounded(Rectangle b, int radius)
        {
            GraphicsPath path = new GraphicsPath();
            int d = Math.Max(2, Math.Min(radius * 2, Math.Min(b.Width, b.Height)));
            path.AddArc(b.X, b.Y, d, d, 180, 90);
            path.AddArc(b.Right - d, b.Y, d, d, 270, 90);
            path.AddArc(b.Right - d, b.Bottom - d, d, d, 0, 90);
            path.AddArc(b.X, b.Bottom - d, d, d, 90, 90);
            path.CloseFigure();
            return path;
        }

        /**
         * The Material 3 Expressive "cookie" (the app's Md3/Shapes.js cookie9): a circle with n soft scallops,
         * r(t) = 1 - depth * (0.5 - 0.5 cos(n t)). The app's logo tile has the same shape.
         */
        private static GraphicsPath Cookie(RectangleF b, int n, double depth)
        {
            const int points = 270;
            PointF[] pts = new PointF[points];
            double cx = b.X + b.Width / 2.0, cy = b.Y + b.Height / 2.0, rx = b.Width / 2.0, ry = b.Height / 2.0;
            for (int i = 0; i < points; i++)
            {
                double t = 2 * Math.PI * i / points - Math.PI / 2;
                double r = 1 - depth * (0.5 - 0.5 * Math.Cos(n * (t + Math.PI / 2)));
                pts[i] = new PointF((float)(cx + rx * r * Math.Cos(t)), (float)(cy + ry * r * Math.Sin(t)));
            }
            GraphicsPath path = new GraphicsPath();
            path.AddPolygon(pts);
            return path;
        }

        private static void FillRound(Graphics g, Rectangle b, int radius, Color fill)
        {
            using (GraphicsPath p = Rounded(b, radius))
            using (SolidBrush br = new SolidBrush(fill)) g.FillPath(br, p);
        }

        private static void StrokeRound(Graphics g, Rectangle b, int radius, Color stroke)
        {
            Rectangle r = new Rectangle(b.X, b.Y, b.Width - 1, b.Height - 1);
            using (GraphicsPath p = Rounded(r, radius))
            using (Pen pen = new Pen(stroke, 1f)) g.DrawPath(pen, p);
        }

        private static void Txt(Graphics g, string text, Font font, Color color, Rectangle box, TextFormatFlags flags)
        {
            TextRenderer.DrawText(g, text, font, box, color, flags | TextFormatFlags.NoPrefix | TextFormatFlags.NoPadding);
        }

        private const TextFormatFlags TLeft = TextFormatFlags.Left | TextFormatFlags.VerticalCenter | TextFormatFlags.EndEllipsis;
        private const TextFormatFlags TCentre = TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter;
        private const TextFormatFlags TWrap = TextFormatFlags.Left | TextFormatFlags.Top | TextFormatFlags.WordBreak;

        // ── Layout: where the clickable things are on the current page ────────────────────

        private void Relayout()
        {
            spots.Clear();
            bool busy = page == PageWorking;
            if (!busy) AddSpot("close", R(820 - 46, 0, 46, 36), delegate { Close(); });
            AddSpot("min", R(820 - (busy ? 46 : 92), 0, 46, 36), delegate { WindowState = FormWindowState.Minimized; });

#if UNINSTALLER
            txtPath.Visible = false;
            if (page == PageOptions)
            {
                AddSpot("keep", R(ContentX, 226, ContentW, 30), delegate { optKeepData = !optKeepData; });
                AddSpot("primary", R(ContentX + ContentW - 168, 452, 168, 44), StartWork);
                AddSpot("cancel", R(ContentX + ContentW - 168 - 104, 452, 92, 44), delegate { Close(); });
            }
            else if (page == PageDone)
            {
                AddSpot("primary", R(ContentX + ContentW - 168, 452, 168, 44), Finish);
            }
#else
            txtPath.Visible = page == PageOptions;
            if (page == PageOptions)
            {
                Rectangle field = PathField();
                txtPath.Width = field.Width - S(24);
                txtPath.Location = new Point(field.X + S(12), field.Y + (field.Height - txtPath.Height) / 2);
                AddSpot("browse", R(ContentX + ContentW - 92, 256, 92, 36), Browse);
                AddSpot("desktop", R(ContentX, 322, ContentW, 30), delegate { optDesktop = !optDesktop; });
                AddSpot("startmenu", R(ContentX, 352, ContentW, 30), delegate { optStartMenu = !optStartMenu; });
                AddSpot("autostart", R(ContentX, 382, ContentW, 30), delegate { optAutostart = !optAutostart; });
                AddSpot("primary", R(ContentX + ContentW - 168, 452, 168, 44), StartWork);
                AddSpot("cancel", R(ContentX + ContentW - 168 - 104, 452, 92, 44), delegate { Close(); });
            }
            else if (page == PageDone)
            {
                AddSpot("launch", R(ContentX, 290, ContentW, 30), delegate { optLaunch = !optLaunch; });
                AddSpot("primary", R(ContentX + ContentW - 168, 452, 168, 44), Finish);
            }
#endif
            Invalidate();
        }

        private void AddSpot(string id, Rectangle bounds, Action click)
        {
            HotSpot s = new HotSpot();
            s.Id = id; s.Bounds = bounds; s.Click = click;
            spots.Add(s);
        }

        private Rectangle PathField() { return R(ContentX, 256, ContentW - 102, 36); }
        private Rectangle WaveArea() { return R(0, 318, PaneWidth, 70); }
        private Rectangle ProgressArea() { return R(ContentX - 8, 160, ContentW + 16, 160); }

        private HotSpot SpotAt(Point p)
        {
            foreach (HotSpot s in spots) if (s.Bounds.Contains(p)) return s;
            return null;
        }

        // ── Mouse and keyboard ────────────────────────────────────────────────────────────

        protected override void OnMouseMove(MouseEventArgs e)
        {
            base.OnMouseMove(e);
            HotSpot s = SpotAt(e.Location);
            string id = s == null ? null : s.Id;
            if (id != hoverId) { hoverId = id; Cursor = id == null ? Cursors.Default : Cursors.Hand; Invalidate(); }
        }

        protected override void OnMouseLeave(EventArgs e)
        {
            base.OnMouseLeave(e);
            if (hoverId != null) { hoverId = null; Invalidate(); }
        }

        protected override void OnMouseDown(MouseEventArgs e)
        {
            base.OnMouseDown(e);
            if (e.Button != MouseButtons.Left) return;
            HotSpot s = SpotAt(e.Location);
            if (s != null) { pressedId = s.Id; Invalidate(); return; }
            // Anywhere else on the top strip or the brand pane drags the window.
            if (e.Y < S(40) || e.X < S(PaneWidth)) { ReleaseCapture(); SendMessage(Handle, 0xA1, 0x2, 0); }
        }

        protected override void OnMouseUp(MouseEventArgs e)
        {
            base.OnMouseUp(e);
            HotSpot s = SpotAt(e.Location);
            string was = pressedId;
            pressedId = null;
            if (s != null && s.Id == was) s.Click();
            Invalidate();
        }

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            if (keyData == Keys.Enter && page != PageWorking)
            {
                foreach (HotSpot s in spots) if (s.Id == "primary") { s.Click(); return true; }
            }
            if (keyData == Keys.Escape && page != PageWorking) { Close(); return true; }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            // Stopping half-way would leave a broken install: the window stays until it is done.
            if (page == PageWorking && e.CloseReason == CloseReason.UserClosing) e.Cancel = true;
#if UNINSTALLER
            // closed with × on the last page: the folder still goes
            if (!e.Cancel && page == PageDone) RemoveFolderAfterExit();
#endif
            base.OnFormClosing(e);
        }

        // ── Painting ──────────────────────────────────────────────────────────────────────

        protected override void OnPaint(PaintEventArgs e)
        {
            Graphics g = e.Graphics;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.InterpolationMode = InterpolationMode.HighQualityBicubic;
            g.PixelOffsetMode = PixelOffsetMode.HighQuality;
            g.TextRenderingHint = TextRenderingHint.ClearTypeGridFit;
            g.Clear(Bg);

            PaintBrandPane(g);
            PaintWindowButtons(g);
#if UNINSTALLER
            if (page == PageOptions) PaintUninstallConfirm(g);
            else if (page == PageWorking) PaintWorking(g, "Removing " + ShownName, "This only takes a few seconds.");
            else PaintUninstallDone(g);
#else
            if (page == PageOptions) PaintOptions(g);
            else if (page == PageWorking) PaintWorking(g, updating ? "Updating " + ShownName : "Installing " + ShownName, "This takes about a minute. You can keep using your PC.");
            else PaintDone(g);
#endif

            using (Pen border = new Pen(Outline, 1f)) g.DrawRectangle(border, 0, 0, ClientSize.Width - 1, ClientSize.Height - 1);
        }

        private void FillGlow(Graphics g, Rectangle pane, Rectangle area, Color c, int alpha)
        {
            using (GraphicsPath gp = new GraphicsPath())
            {
                gp.AddEllipse(area);
                using (PathGradientBrush pgb = new PathGradientBrush(gp))
                {
                    pgb.CenterColor = Color.FromArgb(alpha, c);
                    pgb.SurroundColors = new Color[] { Color.FromArgb(0, c) };
                    Region old = g.Clip;
                    g.SetClip(pane);
                    g.FillPath(pgb, gp);
                    g.Clip = old;
                }
            }
        }

        private void PaintBrandPane(Graphics g)
        {
            Rectangle pane = new Rectangle(0, 0, S(PaneWidth), ClientSize.Height);
            using (LinearGradientBrush br = new LinearGradientBrush(pane, PaneTop, PaneBottom, 70f)) g.FillRectangle(br, pane);

            // Soft light behind the logo, with a touch of the logo's purple low on the pane.
            FillGlow(g, pane, R(PaneWidth / 2 - 160, 0, 320, 320), Glow, 110);
            FillGlow(g, pane, R(-120, 300, 340, 300), GlowPurple, 45);

            // The launcher icon: the mark on a white M3 Expressive cookie, with a soft shadow.
            RectangleF tile = new RectangleF(S(PaneWidth / 2 - 50), S(108), S(100), S(100));
            for (int i = 4; i >= 1; i--)
            {
                RectangleF sh = tile;
                sh.Inflate(S(i * 1.2), S(i * 1.2));
                sh.Offset(0, S(3 + i));
                using (GraphicsPath p = Cookie(sh, 9, 0.075))
                using (SolidBrush b = new SolidBrush(Color.FromArgb(14, 0, 8, 40))) g.FillPath(b, p);
            }
            using (GraphicsPath p = Cookie(tile, 9, 0.075))
            {
                using (SolidBrush white = new SolidBrush(Color.White)) g.FillPath(white, p);
                if (logo != null)
                {
                    Region old = g.Clip;
                    g.SetClip(p, CombineMode.Intersect);
                    RectangleF mark = tile;
                    mark.Inflate(tile.Width * 0.05f, tile.Height * 0.05f);
                    g.DrawImage(logo, mark);
                    g.Clip = old;
                }
            }

            Txt(g, ShownName, fBrand, Color.White, R(0, 222, PaneWidth, 40), TCentre);
            Txt(g, "Music player for Windows", fBody, Color.FromArgb(200, 218, 250), R(0, 262, PaneWidth, 22), TCentre);

            // The moving waveform: a few rounded bars.
            const int bars = 11;
            const double barW = 5, gap = 7;
            double total = bars * barW + (bars - 1) * gap;
            double x0 = (PaneWidth - total) / 2.0, mid = 352;
            using (SolidBrush bar = new SolidBrush(Color.FromArgb(215, 176, 214, 255)))
            {
                for (int i = 0; i < bars; i++)
                {
                    double envelope = 0.45 + 0.55 * Math.Sin(Math.PI * (i + 0.5) / bars);
                    double h = 8 + 34 * envelope * (0.55 + 0.45 * Math.Sin(wavePhase + i * 0.85));
                    Rectangle r = new Rectangle(S(x0 + i * (barW + gap)), S(mid - h / 2), Math.Max(2, S(barW)), Math.Max(2, S(h)));
                    using (GraphicsPath p = Rounded(r, r.Width / 2)) g.FillPath(bar, p);
                }
            }

            Txt(g, "Version " + version, fSmall, Color.FromArgb(150, 170, 215), R(0, 478, PaneWidth, 20), TCentre);
        }

        private void PaintWindowButtons(Graphics g)
        {
            foreach (HotSpot s in spots)
            {
                if (s.Id != "close" && s.Id != "min") continue;
                bool hover = hoverId == s.Id;
                if (hover) using (SolidBrush b = new SolidBrush(s.Id == "close" ? Color.FromArgb(196, 43, 28) : SurfaceHover)) g.FillRectangle(b, s.Bounds);
                int cx = s.Bounds.X + s.Bounds.Width / 2, cy = s.Bounds.Y + s.Bounds.Height / 2, d = S(5);
                using (Pen pen = new Pen(hover ? Color.White : Text2, Math.Max(1f, k)))
                {
                    if (s.Id == "close") { g.DrawLine(pen, cx - d, cy - d, cx + d, cy + d); g.DrawLine(pen, cx - d, cy + d, cx + d, cy - d); }
                    else g.DrawLine(pen, cx - d, cy, cx + d, cy);
                }
            }
        }

        private void PaintFeatures(Graphics g, string[] features, int top)
        {
            for (int i = 0; i < features.Length; i++)
            {
                int y = top + i * 28;
                PaintCheckBadge(g, R(ContentX, y + 3, 18, 18));
                Txt(g, features[i], fBody, Text1, R(ContentX + 28, y, ContentW - 28, 24), TLeft);
            }
        }

        private void PaintOptions(Graphics g)
        {
            Txt(g, updating ? "Update " + ShownName : "Install " + ShownName, fTitle, Text1, R(ContentX, 52, ContentW, 36), TLeft);
            Txt(g, "Everything is included. No administrator rights needed.", fBody, Text2, R(ContentX, 88, ContentW, 22), TLeft);

            PaintFeatures(g, new string[]
            {
                "YouTube Music, JioSaavn, lossless Monochrome and your own files",
                "Synced lyrics, an equalizer and Last.fm scrobbling",
                "Material 3 Expressive design with colour palettes",
            }, 128);

            Txt(g, "INSTALL LOCATION", fLabel, Text3, R(ContentX, 232, ContentW, 18), TLeft);
            Rectangle field = PathField();
            FillRound(g, field, S(8), Surface);
            StrokeRound(g, field, S(8), txtPath.Focused ? Accent : Outline);
            // a window that is never shown (--screenshot) doesn't draw the text box: paint its text instead
            if (screenshotting)
                Txt(g, txtPath.Text, fBody, Text1, new Rectangle(field.X + S(12), field.Y, field.Width - S(24), field.Height), TLeft | TextFormatFlags.PathEllipsis);
            PaintButton(g, "browse", "Browse…", false);

            string space = SpaceLine();
            if (space != null) Txt(g, space, fSmall, Text3, R(ContentX, 296, ContentW, 18), TLeft);

            PaintToggle(g, "desktop", "Add a shortcut to the desktop", optDesktop);
            PaintToggle(g, "startmenu", "Add " + ShownName + " to the Start menu", optStartMenu);
            PaintToggle(g, "autostart", "Start " + ShownName + " when Windows starts", optAutostart);

            if (errorText != null) Txt(g, errorText, fSmall, closeFirst ? Text2 : Danger, R(ContentX, 416, ContentW, 32), TWrap);

            PaintButton(g, "cancel", "Cancel", false);
            PaintButton(g, "primary", closeFirst ? "Close and install" : updating ? "Update" : "Install", true);
        }

        private void PaintWorking(Graphics g, string title, string subtitle)
        {
            Txt(g, title, fTitle, Text1, R(ContentX, 52, ContentW, 36), TLeft);
            Txt(g, subtitle, fBody, Text2, R(ContentX, 88, ContentW, 22), TLeft);

            int shown = (int)Math.Round(shownProgress);
            Txt(g, shown + "%", fHuge, Text1, R(ContentX - 4, 170, ContentW, 76), TLeft);

            Rectangle track = R(ContentX, 262, ContentW, 10);
            FillRound(g, track, track.Height / 2, Surface);
            int w = (int)(track.Width * (shownProgress / 100.0));
            if (w >= track.Height)
            {
                Rectangle fill = new Rectangle(track.X, track.Y, w, track.Height);
                using (GraphicsPath p = Rounded(fill, fill.Height / 2))
                using (LinearGradientBrush br = new LinearGradientBrush(track, AccentDeep, Accent, LinearGradientMode.Horizontal))
                    g.FillPath(br, p);
            }
            Txt(g, progressDetail, fSmall, Text3, R(ContentX, 284, ContentW, 20), TLeft);
        }

        private void PaintTick(Graphics g, string title, string line1, string line2)
        {
            Rectangle badge = R(ContentX, 70, 60, 60);
            using (SolidBrush b = new SolidBrush(Color.FromArgb(40, Accent))) g.FillEllipse(b, badge);
            using (Pen ring = new Pen(Accent, Math.Max(1.5f, 2f * k))) g.DrawEllipse(ring, badge);
            using (Pen tick = new Pen(Accent, Math.Max(2f, 3.2f * k)))
            {
                tick.StartCap = LineCap.Round; tick.EndCap = LineCap.Round; tick.LineJoin = LineJoin.Round;
                g.DrawLines(tick, new Point[] { new Point(S(ContentX + 18), S(101)), new Point(S(ContentX + 27), S(110)), new Point(S(ContentX + 43), S(91)) });
            }
            Txt(g, title, fTitle, Text1, R(ContentX, 152, ContentW, 36), TLeft);
            Txt(g, line1, fBody, Text2, R(ContentX, 190, ContentW, 22), TLeft);
            if (line2 != null) Txt(g, line2, fSmall, Text3, R(ContentX, 216, ContentW, 20), TLeft);
        }

        private void PaintDone(Graphics g)
        {
            string where = optDesktop && optStartMenu ? "You'll find it on your desktop and in the Start menu."
                : optDesktop ? "You'll find it on your desktop."
                : optStartMenu ? "You'll find it in the Start menu."
                : "It is in " + installedDir + ".";
            PaintTick(g, updating ? ShownName + " is up to date" : ShownName + " is installed", where,
                "To remove it later, use Windows Settings → Apps → Installed apps.");
            PaintToggle(g, "launch", "Open " + ShownName + " now", optLaunch);
            PaintButton(g, "primary", "Finish", true);
        }

        private void PaintUninstallConfirm(Graphics g)
        {
            Txt(g, "Uninstall " + ShownName, fTitle, Text1, R(ContentX, 52, ContentW, 36), TLeft);
            Txt(g, "This removes the app, its shortcuts and its entry in Installed apps.", fBody, Text2, R(ContentX, 88, ContentW, 22), TLeft);
            Txt(g, "FOLDER", fLabel, Text3, R(ContentX, 132, ContentW, 18), TLeft);
            Rectangle field = R(ContentX, 154, ContentW, 36);
            FillRound(g, field, S(8), Surface);
            StrokeRound(g, field, S(8), Outline);
            Txt(g, txtPath.Text, fBody, Text1, new Rectangle(field.X + S(12), field.Y, field.Width - S(24), field.Height), TLeft | TextFormatFlags.PathEllipsis);
            PaintToggle(g, "keep", "Keep my settings, sign-ins and downloads", optKeepData);
            Txt(g, optKeepData ? "Installing " + ShownName + " again picks up where you left off."
                               : "Settings, the cache and downloaded songs are deleted too. Sign out in the app first to remove saved sign-ins.",
                fSmall, Text3, R(ContentX, 262, ContentW, 40), TWrap);
            if (errorText != null) Txt(g, errorText, fSmall, Danger, R(ContentX, 396, ContentW, 44), TWrap);
            PaintButton(g, "cancel", "Cancel", false);
            PaintButton(g, "primary", "Uninstall", true);
        }

        private void PaintUninstallDone(Graphics g)
        {
            PaintTick(g, ShownName + " was removed", "Thanks for listening.",
                optKeepData ? "Your settings were kept for next time." : null);
            PaintButton(g, "primary", "Close", true);
        }

        private Rectangle BoundsOf(string id)
        {
            foreach (HotSpot s in spots) if (s.Id == id) return s.Bounds;
            return Rectangle.Empty;
        }

        private void PaintButton(Graphics g, string id, string text, bool primary)
        {
            Rectangle b = BoundsOf(id);
            if (b.IsEmpty) return;
            bool hover = hoverId == id, pressed = pressedId == id;
            if (pressed) b.Offset(0, S(1));
            int radius = b.Height / 2;
            if (primary)
            {
                FillRound(g, b, radius, hover ? AccentHover : Accent);
                Txt(g, text, fBodyBold, OnAccent, b, TCentre);
            }
            else
            {
                FillRound(g, b, radius, hover ? SurfaceHover : Surface);
                StrokeRound(g, b, radius, Outline);
                Txt(g, text, fBody, Text1, b, TCentre);
            }
        }

        private void PaintToggle(Graphics g, string id, string text, bool on)
        {
            Rectangle row = BoundsOf(id);
            if (row.IsEmpty) return;
            int size = S(20);
            Rectangle box = new Rectangle(row.X, row.Y + (row.Height - size) / 2, size, size);
            if (on)
            {
                FillRound(g, box, S(6), Accent);
                using (Pen tick = new Pen(OnAccent, Math.Max(1.6f, 2f * k)))
                {
                    tick.StartCap = LineCap.Round; tick.EndCap = LineCap.Round; tick.LineJoin = LineJoin.Round;
                    g.DrawLines(tick, new PointF[]
                    {
                        new PointF(box.X + size * 0.26f, box.Y + size * 0.52f),
                        new PointF(box.X + size * 0.44f, box.Y + size * 0.70f),
                        new PointF(box.X + size * 0.76f, box.Y + size * 0.32f),
                    });
                }
            }
            else
            {
                FillRound(g, box, S(6), Surface);
                StrokeRound(g, box, S(6), hoverId == id ? Accent : Text3);
            }
            Txt(g, text, fBody, hoverId == id ? Color.White : Text1, new Rectangle(box.Right + S(10), row.Y, row.Width - size - S(10), row.Height), TLeft);
        }

        private void PaintCheckBadge(Graphics g, Rectangle box)
        {
            using (SolidBrush b = new SolidBrush(Color.FromArgb(46, Accent))) g.FillEllipse(b, box);
            using (Pen tick = new Pen(Accent, Math.Max(1.4f, 1.7f * k)))
            {
                tick.StartCap = LineCap.Round; tick.EndCap = LineCap.Round; tick.LineJoin = LineJoin.Round;
                float s = box.Width;
                g.DrawLines(tick, new PointF[]
                {
                    new PointF(box.X + s * 0.28f, box.Y + s * 0.52f),
                    new PointF(box.X + s * 0.44f, box.Y + s * 0.68f),
                    new PointF(box.X + s * 0.73f, box.Y + s * 0.34f),
                });
            }
        }

        // ── Options page actions ──────────────────────────────────────────────────────────

        private static string Megabytes(long bytes)
        {
            double mb = bytes / (1024.0 * 1024.0);
            return mb >= 1024 ? (mb / 1024.0).ToString("0.0") + " GB" : Math.Max(1, (long)Math.Round(mb)) + " MB";
        }

        /** Free space on the drive of [path], or -1 when it can't be told (bad path, network share). */
        private static long FreeSpace(string path)
        {
            try
            {
                string root = Path.GetPathRoot(Path.GetFullPath(path));
                return string.IsNullOrEmpty(root) ? -1 : new DriveInfo(root).AvailableFreeSpace;
            }
            catch { return -1; }
        }

        private string SpaceLine()
        {
            if (payloadBytes < 1024 * 1024) return null;
            long free = FreeSpace(txtPath.Text.Trim());
            string need = "Needs about " + Megabytes(payloadBytes);
            return free < 0 ? need : need + "  ·  " + Megabytes(free) + " free on this drive";
        }

        private void Browse()
        {
            using (FolderBrowserDialog dialog = new FolderBrowserDialog())
            {
                dialog.Description = "Choose the folder " + ShownName + " is installed in";
                try { dialog.SelectedPath = Path.GetDirectoryName(txtPath.Text.Trim()); } catch { }
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                string chosen = dialog.SelectedPath;
                // Always its own folder, so uninstalling never touches other files.
                txtPath.Text = Path.GetFileName(chosen).Equals(AppName, StringComparison.OrdinalIgnoreCase) ? chosen : Path.Combine(chosen, AppName);
            }
        }

        /** Pixel Music processes started from [dir] (a dev build elsewhere is left alone). */
        private static List<Process> RunningFrom(string dir)
        {
            List<Process> found = new List<Process>();
            string root = dir.TrimEnd('\\') + "\\";
            foreach (Process p in Process.GetProcessesByName(Path.GetFileNameWithoutExtension(ExeName)))
            {
                try
                {
                    string path = p.MainModule.FileName;
                    if (path.StartsWith(root, StringComparison.OrdinalIgnoreCase)) found.Add(p);
                }
                catch { }
            }
            return found;
        }

        private static void CloseRunning(string dir)
        {
            foreach (Process p in RunningFrom(dir))
            {
                try
                {
                    // ask nicely first (the app saves its queue and settings on close), then insist
                    if (!p.CloseMainWindow() || !p.WaitForExit(4000)) { p.Kill(); p.WaitForExit(4000); }
                }
                catch { }
            }
        }

        // ── Installing / removing ─────────────────────────────────────────────────────────

        private void ShowProgress(int percent, string detail)
        {
            if (verySilent || !IsHandleCreated) return;
            BeginInvoke((Action)delegate
            {
                progress = percent;
                progressDetail = detail;
                Invalidate(ProgressArea());
            });
        }

        private void StartWork()
        {
            string targetDir = txtPath.Text.Trim();
            try
            {
                if (targetDir.Length == 0 || !Path.IsPathRooted(targetDir)) throw new ArgumentException();
                targetDir = Path.GetFullPath(targetDir).TrimEnd('\\');
            }
            catch
            {
                errorText = "Choose a full folder path, for example C:\\Users\\you\\AppData\\Local\\Programs\\" + AppName + ".";
                Invalidate();
                return;
            }
#if !UNINSTALLER
            long free = FreeSpace(targetDir);
            if (payloadBytes > 0 && free >= 0 && free < payloadBytes + 50L * 1024 * 1024)
            {
                errorText = "Not enough free space on that drive: " + ShownName + " needs about " + Megabytes(payloadBytes) + ", and " + Megabytes(free) + " is free.";
                Invalidate();
                return;
            }
#endif
            // An open Pixel Music keeps its files locked. Interactive: say so once, the second press closes it.
            if (!silent && !closeFirst && RunningFrom(targetDir).Count > 0)
            {
                closeFirst = true;
                errorText = ShownName + " is open. It will be closed so its files can be replaced.";
                Relayout();
                return;
            }

            errorText = null;
            progress = 0;
            shownProgress = 0;
            progressDetail = "Getting ready…";
            page = PageWorking;
            Relayout();

            bool makeDesktop = optDesktop, makeStartMenu = optStartMenu, makeAutostart = optAutostart, keepData = optKeepData;
            Thread worker = new Thread(delegate ()
            {
                try
                {
                    // interactive: the user agreed above; /SILENT: the in-app updater has quit already (or asks with /CLOSEAPPLICATIONS)
                    CloseRunning(targetDir);
#if UNINSTALLER
                    Uninstall(targetDir, keepData);
#else
                    Install(targetDir, makeDesktop, makeStartMenu, makeAutostart);
#endif
                    BeginInvoke((Action)delegate
                    {
                        installedDir = targetDir;
                        progress = 100;
                        page = PageDone;
                        Relayout();
                        if (silent) Finish();
                    });
                }
                catch (Exception ex)
                {
                    string message = ex is UnauthorizedAccessException
                        ? "Windows didn't allow writing to that folder. Choose a folder inside your user profile."
                        : (ex is IOException ? "A file is in use: close " + ShownName + " and try again. (" + ex.Message + ")"
                                              : "That didn't finish: " + ex.Message);
                    BeginInvoke((Action)delegate
                    {
                        errorText = message;
                        page = PageOptions;
                        if (silent) { silent = false; Show(); }
                        Relayout();
                    });
                }
            });
            worker.IsBackground = true;
            worker.SetApartmentState(ApartmentState.STA); // the shortcut COM objects want an STA thread
            worker.Start();
        }

#if !UNINSTALLER
        private void Install(string targetDir, bool makeDesktop, bool makeStartMenu, bool makeAutostart)
        {
            Directory.CreateDirectory(targetDir);
            string root = targetDir + Path.DirectorySeparatorChar;
            Assembly asm = Assembly.GetExecutingAssembly();

            // QML is replaced as a whole: files dropped in the new version must not linger (as pixelmusic.iss did)
            string qml = Path.Combine(targetDir, "qml");
            if (Directory.Exists(qml)) { ShowProgress(1, "Removing the old interface files…"); Directory.Delete(qml, true); }

            using (Stream zipStream = asm.GetManifestResourceStream("payload.zip"))
            {
                if (zipStream == null) throw new InvalidOperationException("this installer is incomplete (the app files are missing). Download it again.");
                using (ZipArchive archive = new ZipArchive(zipStream, ZipArchiveMode.Read))
                {
                    long total = 0, done = 0;
                    foreach (ZipArchiveEntry entry in archive.Entries) total += entry.Length;
                    int shown = -1;
                    foreach (ZipArchiveEntry entry in archive.Entries)
                    {
                        string destPath = Path.GetFullPath(Path.Combine(targetDir, entry.FullName));
                        // Never write outside the install folder, whatever a path in the zip says.
                        if (!destPath.StartsWith(root, StringComparison.OrdinalIgnoreCase)) continue;
                        if (string.IsNullOrEmpty(entry.Name)) Directory.CreateDirectory(destPath);
                        else
                        {
                            Directory.CreateDirectory(Path.GetDirectoryName(destPath));
                            entry.ExtractToFile(destPath, true);
                        }
                        done += entry.Length;
                        // By size, not by file count, so the bar moves evenly; and only when the
                        // number changes, so thousands of small files don't flood the window.
                        int percent = total > 0 ? 2 + (int)(done * 88 / total) : 0;
                        if (percent != shown) { shown = percent; ShowProgress(percent, "Copying " + entry.Name); }
                    }
                }
            }

            ShowProgress(92, "Adding the uninstaller…");
            string uninstaller = Path.Combine(targetDir, "Uninstall.exe");
            ExtractResource(asm, "Uninstall.exe", uninstaller);
            // an earlier Inno Setup install: its uninstaller is replaced by ours (same "Installed apps" entry)
            foreach (string old in new string[] { "unins000.exe", "unins000.dat", "unins000.msg" })
            {
                try { File.Delete(Path.Combine(targetDir, old)); } catch { }
            }

            ShowProgress(95, "Creating shortcuts…");
            string exePath = Path.Combine(targetDir, ExeName);
            CreateShortcuts(targetDir, exePath, makeDesktop, makeStartMenu, makeAutostart);

            ShowProgress(98, "Registering " + ShownName + " with Windows…");
            RegisterUninstall(targetDir, exePath, uninstaller);
            Thread.Sleep(300);
        }

        private static void ExtractResource(Assembly asm, string name, string destination)
        {
            using (Stream s = asm.GetManifestResourceStream(name))
            {
                if (s == null) return;
                using (FileStream fs = File.Create(destination)) s.CopyTo(fs);
            }
        }

        private void RegisterUninstall(string targetDir, string exePath, string uninstPath)
        {
            using (RegistryKey key = Registry.CurrentUser.CreateSubKey(UninstallKey))
            {
                if (key == null) return;
                key.SetValue("DisplayName", ShownName);
                key.SetValue("DisplayVersion", version);
                key.SetValue("DisplayIcon", exePath + ",0");
                key.SetValue("Publisher", Publisher);
                key.SetValue("InstallLocation", targetDir + "\\");
                key.SetValue("UninstallString", "\"" + uninstPath + "\"");
                key.SetValue("QuietUninstallString", "\"" + uninstPath + "\" /SILENT");
                key.SetValue("InstallDate", DateTime.Now.ToString("yyyyMMdd"));
                key.SetValue("NoModify", 1, RegistryValueKind.DWord);
                key.SetValue("NoRepair", 1, RegistryValueKind.DWord);
                int sizeKb = payloadBytes > 0 ? (int)Math.Min(int.MaxValue, payloadBytes / 1024) : 300000;
                key.SetValue("EstimatedSize", sizeKb, RegistryValueKind.DWord);
                // Inno's own values would point at the removed unins000.exe
                foreach (string name in new string[] { "Inno Setup: App Path", "Inno Setup: Icon Group", "Inno Setup: User", "Inno Setup: Setup Version", "Inno Setup: Language", "Inno Setup: Selected Tasks", "Inno Setup: Deselected Tasks" })
                {
                    try { key.DeleteValue(name, false); } catch { }
                }
            }
        }
#endif

        private static string Folder(Environment.SpecialFolder f) { return Environment.GetFolderPath(f); }
        private static string DesktopLink() { return Path.Combine(Folder(Environment.SpecialFolder.DesktopDirectory), ShownName + ".lnk"); }
        private static string StartMenuLink() { return Path.Combine(Folder(Environment.SpecialFolder.Programs), ShownName + ".lnk"); }
        private static string StartupLink() { return Path.Combine(Folder(Environment.SpecialFolder.Startup), ShownName + ".lnk"); }
        // the "Pixel Music" shortcuts of earlier installs (both installers): Start menu folder with the app and
        // Inno's uninstall entry, desktop and Startup links. The pinned taskbar link is left alone (renaming it unpins).
        private static string LegacyDesktopLink() { return Path.Combine(Folder(Environment.SpecialFolder.DesktopDirectory), AppName + ".lnk"); }
        private static string LegacyStartMenuDir() { return Path.Combine(Folder(Environment.SpecialFolder.Programs), AppName); }
        private static string LegacyStartupLink() { return Path.Combine(Folder(Environment.SpecialFolder.Startup), AppName + ".lnk"); }
        private static void RemoveLegacyLinks()
        {
            TryDelete(LegacyDesktopLink());
            TryDelete(LegacyStartupLink());
            try { if (Directory.Exists(LegacyStartMenuDir())) Directory.Delete(LegacyStartMenuDir(), true); } catch { }
        }
        // HKCU\Software\Classes\AppUserModelId\<id>: the name and icon Windows shows for the app's AppUserModelID
        // (media flyout, notifications) when no shortcut tells it. The app writes the same key at start.
        private const string AumidKey = @"Software\Classes\AppUserModelId\" + AppUserModelId;

#if !UNINSTALLER
        private static void CreateShortcuts(string targetDir, string exePath, bool desktop, bool startMenu, bool autostart)
        {
            RemoveLegacyLinks();
            string about = ShownName + " music player";
            if (desktop) Shortcut.Create(DesktopLink(), exePath, targetDir, about, AppUserModelId);
            else TryDelete(DesktopLink());
            // straight in Programs (no folder): one "PixelMusic" entry in the Start menu's All apps
            if (startMenu) Shortcut.Create(StartMenuLink(), exePath, targetDir, about, AppUserModelId);
            else TryDelete(StartMenuLink());
            // autostart: a shortcut in the user's Startup folder, the same one pixelmusic.iss makes
            if (autostart) Shortcut.Create(StartupLink(), exePath, targetDir, about, AppUserModelId);
            else TryDelete(StartupLink());
            try
            {
                using (RegistryKey k = Registry.CurrentUser.CreateSubKey(AumidKey))
                {
                    if (k != null) k.SetValue("DisplayName", ShownName);   // IconUri: the app sets its colourway's .ico
                }
            }
            catch { }
        }
#endif

        private static void TryDelete(string file)
        {
            try { if (File.Exists(file)) File.Delete(file); } catch { }
        }

#if UNINSTALLER
        private void Uninstall(string targetDir, bool keepData)
        {
            string self = Application.ExecutablePath;
            ShowProgress(5, "Removing shortcuts…");
            TryDelete(DesktopLink());
            TryDelete(StartMenuLink());
            TryDelete(StartupLink());
            RemoveLegacyLinks();
            try { Registry.CurrentUser.DeleteSubKeyTree(AumidKey, false); } catch { }

            ShowProgress(10, "Removing " + ShownName + " from Installed apps…");
            try { Registry.CurrentUser.DeleteSubKeyTree(UninstallKey, false); } catch { }

            // every file but this uninstaller; the folder itself goes once this window closes
            string[] files = Directory.Exists(targetDir) ? Directory.GetFiles(targetDir, "*", SearchOption.AllDirectories) : new string[0];
            int shown = -1;
            for (int i = 0; i < files.Length; i++)
            {
                if (string.Equals(files[i], self, StringComparison.OrdinalIgnoreCase)) continue;
                try { File.SetAttributes(files[i], FileAttributes.Normal); File.Delete(files[i]); } catch { }
                int percent = 10 + (int)((i + 1) * 80L / Math.Max(1, files.Length));
                if (percent != shown) { shown = percent; ShowProgress(percent, "Removing " + Path.GetFileName(files[i])); }
            }

            if (!keepData)
            {
                ShowProgress(92, "Removing settings and cache…");
                string local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
                string roaming = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
                // mf-platform dirs: %APPDATA%\pixelmusic\PixelMusic (settings, state), %LOCALAPPDATA%\pixelmusic\PixelMusic (cache, downloads)
                foreach (string d in new string[] { Path.Combine(roaming, "pixelmusic"), Path.Combine(local, "pixelmusic") })
                {
                    // never the install folder itself (it may sit right there) — it goes below, without this exe
                    if (string.Equals(d.TrimEnd('\\'), targetDir, StringComparison.OrdinalIgnoreCase)) continue;
                    try { if (Directory.Exists(d)) Directory.Delete(d, true); } catch { }
                }
            }
            ShowProgress(100, "Done");
            Thread.Sleep(250);
        }

        /** After the window closes: a hidden cmd waits for this exe to exit, then removes the folder. */
        private void RemoveFolderAfterExit()
        {
            if (cleanedUp) return;
            cleanedUp = true;
            string dir = installedDir;
            if (string.IsNullOrEmpty(dir) || !Directory.Exists(dir)) return;
            try
            {
                ProcessStartInfo psi = new ProcessStartInfo();
                psi.FileName = "cmd.exe";
                psi.Arguments = "/c ping 127.0.0.1 -n 3 > NUL & rmdir /s /q \"" + dir + "\"";
                psi.WindowStyle = ProcessWindowStyle.Hidden;
                psi.CreateNoWindow = true;
                psi.UseShellExecute = false;
                psi.WorkingDirectory = Path.GetTempPath();
                Process.Start(psi);
            }
            catch { }
        }
#endif

        private void Finish()
        {
#if UNINSTALLER
            RemoveFolderAfterExit();
#else
            if (optLaunch && (!silent || relaunch))
            {
                try
                {
                    string exePath = Path.Combine(installedDir, ExeName);
                    if (File.Exists(exePath))
                    {
                        ProcessStartInfo psi = new ProcessStartInfo();
                        psi.FileName = exePath;
                        psi.WorkingDirectory = installedDir;
                        psi.UseShellExecute = true;
                        Process.Start(psi);
                    }
                }
                catch { }
            }
            page = PageOptions; // lets the window close
#endif
            Close();
        }

        // ── Entry point ───────────────────────────────────────────────────────────────────

        private void SaveScreenshot(int which, string file)
        {
            waveTimer.Stop();
            page = which;
#if UNINSTALLER
            if (which == PageWorking) { progress = 46; progressDetail = "Removing Qt6Quick.dll"; }
#else
            if (which == PageWorking) { progress = 72; progressDetail = "Copying Qt6Multimedia.dll"; }
#endif
            shownProgress = progress;
            screenshotting = true;
            if (which == PageDone) installedDir = txtPath.Text;
            Relayout();
            IntPtr unused = Handle; // creates the window without showing it
            using (Bitmap bmp = new Bitmap(ClientSize.Width, ClientSize.Height))
            {
                DrawToBitmap(bmp, new Rectangle(Point.Empty, ClientSize));
                bmp.Save(file, ImageFormat.Png);
            }
        }

        /** /VERYSILENT: the same work without a window. */
        private static int RunHidden(string[] args)
        {
            using (SetupForm form = new SetupForm(args))
            {
                string dir = form.txtPath.Text.Trim().TrimEnd('\\');
                try
                {
                    CloseRunning(dir);
#if UNINSTALLER
                    form.Uninstall(dir, true);
                    form.installedDir = dir;
                    form.RemoveFolderAfterExit();
#else
                    form.Install(dir, form.optDesktop, form.optStartMenu, form.optAutostart);
                    form.installedDir = dir;
                    if (form.relaunch) form.Finish();
#endif
                    return 0;
                }
                catch { return 1; }
            }
        }

        [STAThread]
        static int Main(string[] args)
        {
            try { SetProcessDPIAware(); } catch { }
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            if (args.Length >= 3 && args[0] == "--screenshot")
            {
                // further arguments (e.g. /DIR=<folder>) go to the form, to picture a fresh install or another folder
                string[] rest = new string[args.Length - 3];
                Array.Copy(args, 3, rest, 0, rest.Length);
                using (SetupForm form = new SetupForm(rest)) form.SaveScreenshot(int.Parse(args[1]), args[2]);
                return 0;
            }
            foreach (string a in args)
            {
                if (a.ToUpperInvariant() == "/VERYSILENT") return RunHidden(args);
            }
            Application.Run(new SetupForm(args));
            return 0;
        }
    }

    /** A .lnk with the app's AppUserModelID (taskbar grouping, media overlay, notifications), through the shell. */
    internal static class Shortcut
    {
        [ComImport, Guid("00021401-0000-0000-C000-000000000046")]
        private class CShellLink { }

        [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("000214F9-0000-0000-C000-000000000046")]
        private interface IShellLinkW
        {
            void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder pszFile, int cch, IntPtr pfd, uint fFlags);
            void GetIDList(out IntPtr ppidl);
            void SetIDList(IntPtr pidl);
            void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder pszName, int cch);
            void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string pszName);
            void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder pszDir, int cch);
            void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string pszDir);
            void GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder pszArgs, int cch);
            void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string pszArgs);
            void GetHotkey(out short pwHotkey);
            void SetHotkey(short wHotkey);
            void GetShowCmd(out int piShowCmd);
            void SetShowCmd(int iShowCmd);
            void GetIconLocation([Out, MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder pszIconPath, int cch, out int piIcon);
            void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string pszIconPath, int iIcon);
            void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string pszPathRel, uint dwReserved);
            void Resolve(IntPtr hwnd, uint fFlags);
            void SetPath([MarshalAs(UnmanagedType.LPWStr)] string pszFile);
        }

        [StructLayout(LayoutKind.Sequential, Pack = 4)]
        private struct PropertyKey
        {
            public Guid fmtid;
            public uint pid;
        }

        [StructLayout(LayoutKind.Explicit)]
        private struct PropVariant
        {
            [FieldOffset(0)] public ushort vt;
            [FieldOffset(8)] public IntPtr pointer;
            [FieldOffset(8)] public long padding;
        }

        [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
        private interface IPropertyStore
        {
            void GetCount(out uint cProps);
            void GetAt(uint iProp, out PropertyKey pkey);
            void GetValue(ref PropertyKey key, out PropVariant pv);
            void SetValue(ref PropertyKey key, ref PropVariant pv);
            void Commit();
        }

        [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("0000010b-0000-0000-C000-000000000046")]
        private interface IPersistFile
        {
            void GetClassID(out Guid pClassID);
            [PreserveSig] int IsDirty();
            void Load([MarshalAs(UnmanagedType.LPWStr)] string pszFileName, uint dwMode);
            void Save([MarshalAs(UnmanagedType.LPWStr)] string pszFileName, [MarshalAs(UnmanagedType.Bool)] bool fRemember);
            void SaveCompleted([MarshalAs(UnmanagedType.LPWStr)] string pszFileName);
            void GetCurFile([MarshalAs(UnmanagedType.LPWStr)] out string ppszFileName);
        }

        public static void Create(string link, string target, string workDir, string description, string appId)
        {
            IShellLinkW sl = (IShellLinkW)new CShellLink();
            sl.SetPath(target);
            sl.SetWorkingDirectory(workDir);
            sl.SetDescription(description);
            sl.SetIconLocation(target, 0);
            try
            {
                IPropertyStore store = (IPropertyStore)sl;
                PropertyKey key = new PropertyKey();
                key.fmtid = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"); // PKEY_AppUserModel_ID
                key.pid = 5;
                PropVariant value = new PropVariant();
                value.vt = 31; // VT_LPWSTR
                value.pointer = Marshal.StringToCoTaskMemUni(appId);
                try { store.SetValue(ref key, ref value); store.Commit(); }
                finally { Marshal.FreeCoTaskMem(value.pointer); }
            }
            catch { }
            ((IPersistFile)sl).Save(link, true);
            Marshal.FinalReleaseComObject(sl);
        }
    }
}

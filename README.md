# Munseo (문서)

> 예전 이름은 **HWP2PDF**입니다(0.1.x). 0.1.x를 쓰고 있다면 앱 안의 자동 업데이트로 Munseo가 되며, 설정도 그대로 옮겨집니다.

HWP/HWPX 문서를 끌어다 놓으면 **PDF/A-2u**(텍스트 보존), PDF, Word(DOCX), Markdown, Text, HTML로 변환하는 macOS·Windows 네이티브 앱입니다.
문서 해석과 렌더링은 [rhwp](https://github.com/edwardkim/rhwp)(Rust)를 라이브러리로 직접 링크하고, UI는 egui/eframe을 씁니다.

## 설치

**macOS** (터미널에 한 줄):

```sh
curl -fsSL https://raw.githubusercontent.com/ziozzang/munseo/main/install.sh | sh
```

**Windows** (PowerShell에 한 줄):

```powershell
irm https://raw.githubusercontent.com/ziozzang/munseo/main/install.ps1 | iex
```

- 두 스크립트 모두 최신 릴리스를 받아 `SHA256SUMS`로 검증한 뒤 설치합니다.
  - macOS: `/Applications/Munseo.app`
  - Windows: `%LOCALAPPDATA%\Programs\Munseo`, 시작 메뉴 바로가기 포함
- 앱은 Apple 공증이나 Windows 코드 서명 없이 배포됩니다.
  - 브라우저로 받으면 macOS가 "손상되었거나 확인되지 않은 개발자"라며 막고, Windows는 SmartScreen 경고를 띄웁니다.
  - 위의 한 줄 설치로 받으면 이 표시가 붙지 않아 바로 열립니다.
- 브라우저로 [Releases](https://github.com/ziozzang/munseo/releases)에서 받았다면, 처음 한 번만 다음 중 하나를 하세요.
  - macOS: `xattr -dr com.apple.quarantine /Applications/Munseo.app`
  - Windows: 경고 창에서 "추가 정보 → 실행"을 누릅니다.

## 기능

- **연결 프로그램 등록**: Finder "다음으로 열기"(macOS)와 탐색기 "연결 프로그램" 목록에 나옵니다.
  Windows에서는 우클릭 메뉴에 "Munseo로 변환"도 생깁니다(`install.ps1`로 설치한 경우).
  한컴오피스 같은 기본 앱은 바꾸지 않습니다.
- **드래그앤드롭**: 파일을 놓으면 그 파일을, 폴더를 놓으면 하위 폴더까지 모든 `.hwp`/`.hwpx`를 변환합니다.
  숨김 파일과 한글 잠금 파일(`~$…`)은 건너뜁니다. "파일 선택…" / "폴더 선택…" 버튼으로도 고를 수 있습니다.
- **출력 포맷(드롭다운)**
  | 포맷 | 결과 |
  |---|---|
  | PDF/A-2u (기본) | ISO 19005-2 Level U. 폰트 임베드, 모든 글자에 유니코드 매핑, XMP·sRGB OutputIntent 포함 |
  | PDF | rhwp 출력 그대로(텍스트 임베드) |
  | Word (.docx) | 문단·글자 서식, 문단 번호·글머리표, 표(병합 셀), 그림(WMF 차트 포함), 머리말·꼬리말·쪽 번호, 각주·미주, 다단, 하이퍼링크, **수식은 Word 수식(편집 가능)** |
  | Markdown | 문서 전체를 `.md` 하나로. 수식은 LaTeX(`$…$`), 이미지는 `<이름>_assets/`에 저장 |
  | Text | 문서 전체를 UTF-8 `.txt` 하나로. 수식은 LaTeX |
  | HTML | 페이지별 HTML을 `.html` 하나로 |
- **원본 삭제 체크박스**: 변환에 **성공한** 파일만 원본을 휴지통(Windows: 휴지통)으로 옮깁니다. 영구 삭제가 아니라서 되살릴 수 있습니다.
- **진행 표시**: 전체 진행률 막대와 상태 줄(변환 중·대기·성공·실패 개수), 파일마다 진행 막대와
  지금 하는 일("53/105쪽 PDF로", "문단 1200/4800 옮기는 중"), 경과 시간, 끝난 뒤 쪽수·걸린 시간.
  동시에 최대 4개 파일을 변환합니다.
- 결과 파일은 원본과 같은 폴더에 같은 이름으로 저장됩니다(같은 이름이 있으면 덮어씀).
- 포맷·체크박스 설정은 다음 실행 때도 유지됩니다.

## LLM 입력용 텍스트 품질

rhwp의 SVG 렌더러는 글자를 한 글자씩 배치하면서 공백 문자를 그리지 않아서, 원래 PDF에는 띄어쓰기가
글자 간격으로만 남습니다. 그러면 추출기마다 띄어쓰기를 추측합니다(PyMuPDF/pdfminer는 "2021년평가"처럼 붙이고,
macOS PDFKit은 한 글자마다 띄움). 이 앱은 `patches/rhwp-hwp2pdf.patch`로 공백 자리에
**보이지 않는 실제 공백 글리프(U+0020)**를 넣어, 추출기가 원문 그대로의 띄어쓰기를 읽게 합니다.
화면에 보이는 모양은 바뀌지 않습니다.

| 추출기 | 결과 (예: 목차 한 줄) |
|---|---|
| PyMuPDF / pymupdf4llm | `Ⅲ. 주요업무 추진계획` (원문과 같음) |
| pdfminer / pdfplumber / unstructured | `Ⅲ. 주요업무 추진계획` (원문과 같음) |
| pypdf (LangChain `PyPDFLoader`), macOS 미리보기 | `Ⅲ. 주 요 업 무  추 진 계 획` |

pypdf와 PDFKit은 글자 사이가 1~2px만 벌어져도 공백을 넣습니다. 문서에 지정된 한글 폰트가 없어
대체 폰트로 그리면 글자 폭이 조금 달라져 이런 틈이 생깁니다. 원래 폰트(함초롬 등)를 설치하면 줄어듭니다.
**LLM에 넣는 것이 목적이면 Markdown이나 Text 출력이 가장 깨끗합니다.** 레이아웃을 거치지 않고 문서 모델에서
바로 텍스트를 뽑고, Markdown은 표를 표로 유지합니다. 한컴 전용 문자(PUA)도 표준 유니코드로 바꿉니다.

옛한글 음절(`ᄒᆞᆫ`)은 글꼴이 조각 글리프 여러 개로 그리는데, svg2pdf는 첫 조각에만 원문을 연결합니다.
PDF/A-2u는 모든 글리프에 유니코드가 있어야 하므로, 나머지 조각은 보이지 않는 U+200B에 연결합니다(추출되는 글은 그대로).
rhwp가 글꼴 처리에 실패한 쪽을 글자 윤곽선으로 바꿔 넣는 경우(그 쪽은 텍스트가 없음)는 결과에 ⚠ 경고로 표시합니다.

한글 폰트가 없는 PC에서도 글자가 빠지지 않도록 Noto Sans KR(OFL)을 앱에 내장해 폴백으로 씁니다.
한글 문서가 지정한 폰트(함초롬, HY 계열 등)가 설치돼 있으면 그 폰트가 우선합니다.

## 수식 → LaTeX

한글 수식은 LaTeX가 아니라 한컴 고유 문법(`a over b`, `sqrt {x}`, `sum _{i=1} ^{n}`)으로 저장됩니다.
rhwp의 수식 파서로 트리를 만든 뒤 LaTeX(Markdown·Text)와 Word 수식 OMML(DOCX)로 옮깁니다.
rhwp 파서가 한글 수식 편집기와 다르게 묶는 부분은 `src/equation.rs`에서 바로잡습니다.

| 한글 수식 | 변환 결과 |
|---|---|
| `x = {-b +- sqrt {b^2 - 4ac}} over {2a}` | `x=\frac{-b \pm \sqrt{b^{2}-4 ac}}{2 a}` |
| `lim _{h -> 0} {f(2+h)-f(2)} over h` | `\lim_{h \rightarrow 0}\frac{f(2+h)-f(2)}{h}` |
| `x_i^2` (아래·위첨자 동시) | `x_{i}^{2}` |
| `n CHOOSE k` | `\binom{n}{k}` |
| `rm CH_3 COOH` (`rm`은 다음 전환까지 이어지는 모드) | `\mathrm{{CH}_{3}COOH}` |

검증: rhwp 수식 샘플 259개와 수능형 수학·과학 시험지의 수식 924개를 KaTeX로 렌더해 모두 통과했습니다.
문법 오류가 없다는 뜻이고, 모든 수식의 의미를 사람이 대조한 것은 아닙니다.

## Word(DOCX) 변환

rhwp에는 DOCX 내보내기가 없어서 `src/docx.rs`가 rhwp 문서 모델을 WordprocessingML로 직접 씁니다.
조판은 Word에 맡기므로 쪽 수·줄 바꿈은 원본과 다릅니다. Microsoft Word(macOS)에서 샘플 8개가 모두 복구 안내 없이 열리는 것을 확인했습니다.

- 문단 번호·개요 번호·글머리표는 한글이 조판 때 붙이는 것이라 본문에 없습니다. rhwp 조판기와 같은 규칙으로 계산해 넣습니다
  (rhwp PDF와 비교: hwpspec 1194/1228 문단 일치, 불일치 34건은 옛한글 추출 차이).
- 쪽 번호와 전체 쪽수는 Word의 PAGE/NUMPAGES 필드로 넣어 Word가 매깁니다.
- 외부 링크(http·https·mailto)만 하이퍼링크로 옮기고, 문서 안 책갈피와 `javascript:`는 버립니다.
- 목차의 "……… 12"처럼 점을 글자로 늘어놓은 줄, 칸 오른쪽 끝의 점선 탭은 Word 점선 탭으로 바꿔 줄이 넘치지 않게 합니다.
- 크기 머리글이 없는 WMF 그림(한글 차트에 흔함)에는 머리글을 붙여 Word가 그리게 합니다.

아직 옮기지 않는 것: 차트 개체(그림으로 저장된 차트는 옮겨짐), OLE 개체, 글상자·도형의 위치(글상자 안의 글은 그 자리에 문단으로 풀어 넣음),
떠 있는 그림의 위치(모두 글자처럼 줄 안에 배치).

## 빌드

필요: Rust 1.93 이상, git

```sh
./scripts/fetch-rhwp.sh        # rhwp 를 고정 커밋으로 받아 third_party/rhwp 에 두고 패치 적용
cargo build --release          # target/release/munseo
```

### macOS 앱 번들(.app)

```sh
./scripts/bundle-macos.sh      # dist/Munseo.app
```

### Windows

Windows에서 같은 명령으로 빌드합니다(`scripts/fetch-rhwp.sh`는 Git Bash에서 실행).
릴리스 빌드는 콘솔 창 없이 GUI로 뜨고, 실행 파일에 아이콘이 들어갑니다. 결과: `target\release\munseo.exe`.

### GitHub Actions

`.github/workflows/build.yml`이 브랜치에 푸시할 때마다 macOS와 Windows(x64)에서 테스트·빌드·변환 스모크 테스트를 합니다.
macOS는 arm64 + x86_64 유니버설로 빌드합니다. 결과물은 다음 이름으로 올립니다.

- `Munseo_<버전>_macos_universal.zip`
- `Munseo_<버전>_windows_x86_64.exe`

CI는 빌드와 테스트 확인용입니다. 릴리스는 아래 "릴리스 절차"대로 로컬에서 만듭니다.

## 자동 업데이트

[sugyeol](https://github.com/ziozzang/sugyeol)과 같은 방식으로 GitHub Releases를 씁니다.

- **확인 시점:** 앱을 켤 때 하루 한 번 `api.github.com/repos/ziozzang/munseo/releases/latest`를 확인합니다.
  - 옵션 줄 오른쪽의 **업데이트 확인**으로 직접 확인할 수도 있습니다.
  - 자동 확인을 끄려면 환경 변수 `Munseo_NO_UPDATE_CHECK`를 설정하세요.
- **알림:** 새 버전이 있으면 창 위에 알림이 뜹니다(업데이트 후 다시 시작 / 나중에 / 이 버전 건너뛰기).
  - 묻지 않고 설치하지는 않습니다.
  - 변환이 진행 중일 때는 업데이트 버튼이 꺼집니다.
- **설치 순서:**
  1. 플랫폼용 파일을 내려받으면서 SHA-256을 계산하고, 릴리스의 `SHA256SUMS`와 비교합니다.
  2. 앱을 교체한 뒤 다시 실행합니다.
     - macOS: `.app` 번들을 통째로 바꿉니다. 받은 번들의 번들 ID와 버전도 확인합니다.
     - Windows: 실행 중인 exe를 `.old`로 옮기고 새 exe를 그 자리에 둡니다. `.old` 파일은 다음 실행 때 지웁니다.
- **명령줄:**
  - `munseo update --check`: 새 버전이 있는지 확인만 합니다.
  - `munseo update`: 새 버전을 설치합니다. 창을 띄우지 않고 교체만 합니다.
  - `munseo --version`: 현재 버전을 보여 줍니다.
- 앱이 있는 폴더에 쓸 권한이 있어야 합니다.

## 릴리스 절차

릴리스 파일은 CI 없이 이 Mac에서 직접 빌드해 올립니다.
macOS는 유니버설로, Windows는 mingw로 크로스 빌드합니다.

```sh
brew install rustup mingw-w64     # 처음 한 번
rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-gnu
```

1. `Cargo.toml`의 `version`과 `RELEASE_NOTES.md`를 고치고 커밋합니다.
2. `./scripts/release.sh`를 실행합니다. `out/`에 두 파일과 `SHA256SUMS`를 만들고 검증합니다.
3. `./scripts/release.sh --publish`를 실행합니다(`$GITHUB_TOKEN` 필요). `v<버전>` 태그를 푸시하고 릴리스를 만들어 파일을 올립니다.

## 명령줄 사용

인자를 주면 창 없이 변환합니다.

```sh
munseo [--format pdfa|pdf|docx|md|txt|html] [--delete-original] <파일|폴더>...
```

## 검증

PDF/A 적합성은 [veraPDF](https://verapdf.org)로 확인했습니다.

```sh
verapdf --flavour 2u --format text 결과.pdf
```

## 구성

```
src/main.rs      진입점 (인자 없으면 GUI, 있으면 CLI)
src/gui.rs       드래그앤드롭 창, 작업 큐(동시 작업 최대 4개)
src/batch.rs     파일/폴더 수집, 변환 + 원본 휴지통 이동
src/convert.rs   rhwp 호출 (PDF/Markdown/Text/HTML/DOCX)
src/docx.rs      rhwp 문서 모델 → DOCX
src/equation.rs  한글 수식 파싱 + 한글 문법에 맞춘 트리 보정
src/latex.rs     수식 → LaTeX
src/omml.rs      수식 → Word 수식(OMML)
src/pdfa.rs      PDF/A-2u 후처리 (XMP, OutputIntent, /ID, 금지 키 제거)
src/icc.rs       sRGB ICC v2 프로파일 생성
src/fonts.rs     내장 Noto Sans KR
src/update.rs    GitHub Releases 자동 업데이트
assets/icon/     앱 아이콘 (png, icns, ico)
build.rs         Windows 실행 파일 아이콘
patches/         rhwp 패치 (공백 글리프, 텍스트 추출 시 수식 표식)
install.sh       macOS 한 줄 설치 스크립트
install.ps1      Windows 한 줄 설치 스크립트
scripts/release.sh  로컬 릴리스 빌드(macOS 유니버설 + Windows 크로스)와 업로드
```

## 라이선스

MIT (`LICENSE`). rhwp도 MIT입니다. 내장 폰트 Noto Sans KR은 SIL Open Font License 1.1(`assets/fonts/NotoSansKR-OFL.txt`)을 따릅니다.

# UI/UX implementation specification

## Layout và design system

React shell có desktop sidebar240px,content max1440px,page gutters24px; dưới768px dùng navigation drawer và gutters16px. Đây là proposed design tokens,không phải approved visual prototype. One global header,one h1 per route. Font stack system sans; body14–16px,line-height1.5; headings20–28px; spacing4/8/12/16/24/32; controls tối thiểu44x44CSSpx cho primary mobile actions. Color tokens semantic light/dark; contrast theo WCAG AA. Không hardcode brand colors trong domain plugins.

Common primitives:Button,Dialog,AlertDialog,Sheet,Table,Pagination,Combobox,Toast,FormField,EmptyState,ErrorState,Skeleton,StatusBadge,CodeView,DiffViewer. UI plugin consume chung package,React singleton peer dependency. Tailwind classes static hoặc bundled CSS; khai báo workspace source để production purge không mất style. Dynamic remote class không được coi tự tồn tại.

## Navigation và states

Screen inventory machine-readable ở `screens.json`. Mỗi screen có route,actor,actions và source feature. Bắt buộc loading/empty/error/offline/stale/forbidden/success; thêm saving/conflict/unknown/pending cho màn ghi. Permission UI không thay backend authorization. Readonly role có status rõ,không nút fake enabled.

List search debounce300ms proposed,cancel obsolete requests và ignore late results bằng request version. Filter reset cursor; empty search và empty filtered result có text khác. Detail deep link after login giữ target nếu được quyền. Browser Back không làm reset draft hoặc đổi active tenant ngầm. Không dùng notification click để approve.

## Review/editor

Review luôn hiển thị immutable before/after,target/account,content digest,evidence refs,risk,known/unknown cost và approval expiry. Studio có sidebar evidence và revision diff; narrow viewport chuyển tabs bảo toàn selection. Manual blocks giữ locked indicator. Autosave hiển thị Saving/Saved/Unsaved/Conflict với version; không optimistic Published toast. Destructive action xác nhận target và reason; cancel default focus safe.

## Accessibility

Dialog dùng semantics thực,focus trap hợp lệ,Escape/close theo safety,focus return vào trigger; errors liên kết input; live region cho async status không spam. Loading `aria-busy`;table headers đúng;chart có data table alternative;unknown không chỉ phân biệt màu. Keyboard đạt mọi action. 200% text/zoom không che footer controls. Reduce motion tắt nonessential animation. Toast không là nơi duy nhất thông báo lỗi.

## Evidence

Playwright captures light/dark và top/end cho mỗi state được chọn; viewport320,390,768,1440; text100/200%; input keyboard/touch. Screenshots giữ candidate hash và fixture state IDs. Axe không thay manual screen-reader/focus review. Không đặt pixel-percentage parity khi chưa có approved prototype. File ZIP này không chứa ảnh nghiệm thu sản phẩm vì sản phẩm chưa chạy.

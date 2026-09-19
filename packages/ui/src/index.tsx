import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { Slot } from '@radix-ui/react-slot';
import * as Dialog from '@radix-ui/react-dialog';
import { cva, type VariantProps } from 'class-variance-authority';
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) { return twMerge(clsx(inputs)); }
const button = cva('button', { variants: { variant: {
  primary: 'button-primary', outline: 'button-outline', ghost: 'button-ghost',
} }, defaultVariants: { variant: 'outline' } });
export const Button = forwardRef<HTMLButtonElement,
  ButtonHTMLAttributes<HTMLButtonElement> & VariantProps<typeof button> & { asChild?: boolean }
>(function Button({ className, variant, asChild = false, ...props }, ref) {
  const Component = asChild ? Slot : 'button';
  return <Component ref={ref} className={cn(button({ variant }), className)} {...props} />;
});

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <section className={cn('card', className)}>{children}</section>;
}
export function Badge({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'warning' | 'positive' }) {
  return <span className={cn('badge', `badge-${tone}`)}>{children}</span>;
}

export function NavigationSheet({ open, onOpenChange, onCloseAutoFocus, children }: {
  open: boolean; onOpenChange: (value: boolean) => void;
  onCloseAutoFocus?: (event: Event) => void; children: ReactNode;
}) {
  return <Dialog.Root open={open} onOpenChange={onOpenChange}>
    <Dialog.Portal>
      <Dialog.Overlay className="sheet-overlay" />
      <Dialog.Content className="sheet-content" onCloseAutoFocus={onCloseAutoFocus}>
        <div className="sheet-title-row">
          <Dialog.Title>Điều hướng</Dialog.Title>
          <Dialog.Close asChild><Button aria-label="Đóng điều hướng">Đóng</Button></Dialog.Close>
        </div>
        <Dialog.Description className="sr-only">Các mô-đun trong workspace Masonwing.</Dialog.Description>
        {children}
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

export type DataState = 'loading' | 'empty' | 'error' | 'offline' | 'stale' | 'forbidden' | 'not-implemented' | 'outcome-unknown';
const stateText: Record<DataState, { title: string; detail: string }> = {
  loading: { title: 'Đang tải dữ liệu', detail: 'Đang đọc trạng thái từ dịch vụ.' },
  empty: { title: 'Chưa có dữ liệu', detail: 'Kết nối nguồn hoặc nhập dữ liệu được cấp quyền để bắt đầu.' },
  error: { title: 'Không tải được dữ liệu', detail: 'Dịch vụ chưa trả về kết quả. Dữ liệu cũ không được coi là kết quả mới.' },
  offline: { title: 'Mất kết nối', detail: 'Kiểm tra kết nối rồi tải lại dữ liệu.' },
  stale: { title: 'Dữ liệu cần cập nhật', detail: 'Kiểm tra trạng thái đồng bộ trước khi đưa ra quyết định.' },
  forbidden: { title: 'Không có quyền truy cập', detail: 'Kiểm tra quyền hiện hành của bạn trong workspace.' },
  'not-implemented': { title: 'Sẵn sàng để triển khai', detail: 'Hợp đồng và kết quả kiểm thử kỳ vọng đã có. Handler nghiệp vụ chưa được triển khai.' },
  'outcome-unknown': { title: 'Kết quả chưa xác định', detail: 'Cần đối soát với nhà cung cấp trước khi quyết định bước tiếp theo. Không gửi lại thao tác.' },
};
export function DataSurface({ state, retry }: { state: DataState; retry?: () => void }) {
  const text = stateText[state];
  return <div className={cn('data-surface', `data-${state}`)}
    role={state === 'error' ? 'alert' : 'status'} aria-busy={state === 'loading'}>
    <h2>{text.title}</h2><p>{text.detail}</p>
    {retry && ['error', 'offline', 'stale'].includes(state)
      ? <Button onClick={retry}>Tải lại dữ liệu</Button> : null}
  </div>;
}

import { Component, computed, effect, inject, signal } from '@angular/core';
import { ActivatedRoute } from '@angular/router';
import { toSignal } from '@angular/core/rxjs-interop';
import { HttpClient } from '@angular/common/http';
import { MatButtonModule } from '@angular/material/button';
import { firstValueFrom } from 'rxjs';
import { GetDiagnosticsResponse, ReadDiagnosticResponse } from '../health-response';

type Page = Extract<GetDiagnosticsResponse, { artifacts: unknown }>;
type Chunk = Extract<ReadDiagnosticResponse, { text: unknown }>;
type Artifact = Page['artifacts'][number];

@Component({
  imports: [MatButtonModule],
  selector: 'app-diagnostic-panel',
  styles: `
    :host {
      display: block;
      min-width: 0;
    }
    pre {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    p {
      overflow-wrap: anywhere;
    }
    .actions {
      display: flex;
      flex-wrap: wrap;
      gap: 0.5rem;
    }
  `,
  template: `
    <section class="ui-panel" aria-label="失败诊断与报告">
      <h3>诊断与报告</h3>
      <button matButton="outlined" [disabled]="busy()" (click)="load(0)">刷新诊断清单</button>
      @if (error()) {
        <p role="alert">{{ error() }}</p>
      }
      @if (busy()) {
        <p role="status">正在读取…</p>
      }
      @for (artifact of artifacts(); track artifact.artifact_id) {
        <details>
          <summary>{{ artifact.purpose }} · {{ state(artifact.availability) }}</summary>
          <p>
            阶段 {{ artifact.binding.phase }} · 验证代次 {{ artifact.binding.generation }} · 调用
            {{ artifact.binding.identity.invocation_id }}
          </p>
          <p>
            候选 {{ artifact.binding.candidate?.sha ?? '尚无候选' }} · 保留
            {{ artifact.retained_bytes }} 字节，脱敏内容 {{ artifact.export_bytes }} 字节。
          </p>
          @if (artifact.reason) {
            <p>{{ artifact.reason }}</p>
          }
          <div class="actions">
            @if (readable(artifact)) {
              <button matButton [disabled]="busy()" (click)="read(artifact, 0)">
                查看脱敏内容
              </button>
              <button matButton [disabled]="busy()" (click)="download(artifact)">
                下载已保留内容
              </button>
            }
          </div>
        </details>
      } @empty {
        @if (!busy() && !error()) {
          <p>暂无诊断附件。</p>
        }
      }
      @if (next(); as cursor) {
        <button matButton [disabled]="busy()" (click)="load(cursor)">更多附件</button>
      }
      @if (chunk(); as page) {
        <section aria-label="脱敏诊断内容">
          <h4>{{ page.artifact.purpose }}</h4>
          <p>
            字节 {{ page.offset }}–{{ page.next }} / {{ page.artifact.export_bytes }} ·
            {{ page.end ? '已到末尾' : '后续内容可继续读取' }}
          </p>
          <pre>{{ page.text }}</pre>
          @if (!page.end) {
            <button matButton [disabled]="busy()" (click)="read(page.artifact, page.next)">
              下一段
            </button>
          }
        </section>
      }
    </section>
  `,
})
export class DiagnosticPanel {
  private readonly route = inject(ActivatedRoute);
  private readonly params = toSignal(this.route.paramMap, {
    initialValue: this.route.snapshot.paramMap,
  });
  readonly requirementId = computed(() => Number(this.params().get('id')));
  private readonly http = inject(HttpClient);
  readonly artifacts = signal<Artifact[]>([]);
  readonly next = signal<number | null>(null);
  readonly chunk = signal<Chunk | null>(null);
  readonly busy = signal(false);
  readonly error = signal('');
  private version = 0;
  private readonly chunkLimit = 8192;
  constructor() {
    effect(() => {
      this.requirementId();
      this.chunk.set(null);
      this.artifacts.set([]);
      this.next.set(null);
      void this.load(0);
    });
  }
  async load(after: number) {
    const version = ++this.version;
    this.busy.set(true);
    this.error.set('');
    try {
      const page = await firstValueFrom(
        this.http.get<GetDiagnosticsResponse>(
          `/api/requirements/${this.requirementId()}/diagnostics/${after}`,
        ),
      );
      if ('error' in page) throw new Error(page.error);
      if (version !== this.version) return;
      this.artifacts.set(after === 0 ? page.artifacts : [...this.artifacts(), ...page.artifacts]);
      this.next.set(page.next);
    } catch {
      if (version === this.version) this.error.set('诊断清单暂不可读，请刷新后重试。');
    } finally {
      if (version === this.version) this.busy.set(false);
    }
  }
  state(value: Artifact['availability']) {
    return {
      available: '完整保留',
      partial: '部分保留',
      missing: '内容缺失',
      expired: '已过期',
      corrupt: '摘要不符，读取已拒绝',
    }[value];
  }
  readable(artifact: Artifact) {
    return ['available', 'partial'].includes(artifact.availability);
  }
  private async readChunk(artifact: Artifact, offset: number): Promise<Chunk> {
    const page = await firstValueFrom(
      this.http.get<ReadDiagnosticResponse>(
        `/api/requirements/${this.requirementId()}/diagnostic-artifacts/${artifact.artifact_id}/${offset}/${this.chunkLimit}`,
      ),
    );
    if ('error' in page) throw new Error(page.error);
    this.checkIdentity(artifact, page);
    this.checkRange(page, offset);
    return page;
  }
  private checkIdentity(artifact: Artifact, page: Chunk) {
    if (
      page.artifact.artifact_id !== artifact.artifact_id ||
      page.artifact.export_bytes !== artifact.export_bytes ||
      page.artifact.export_sha256 !== artifact.export_sha256
    )
      throw new Error('读取身份不符');
  }
  private checkRange(page: Chunk, offset: number) {
    if (page.offset !== offset || page.next < offset || page.next > page.artifact.export_bytes)
      throw new Error('读取范围不符');
    if (
      page.end !== (page.next === page.artifact.export_bytes) ||
      (!page.end && page.next === offset)
    )
      throw new Error('读取进度不符');
  }
  async read(artifact: Artifact, offset: number) {
    const version = this.version;
    this.busy.set(true);
    this.error.set('');
    try {
      const page = await this.readChunk(artifact, offset);
      if (version === this.version) this.chunk.set(page);
    } catch {
      if (version === this.version) this.error.set('内容缺失、过期或权限已变化，请刷新诊断清单。');
    } finally {
      if (version === this.version) this.busy.set(false);
    }
  }
  async download(artifact: Artifact) {
    const version = this.version;
    this.busy.set(true);
    this.error.set('');
    try {
      const bytes = await this.full(artifact);
      if (version !== this.version) return;
      const url = URL.createObjectURL(new Blob([bytes], { type: 'text/plain;charset=utf-8' }));
      const link = document.createElement('a');
      link.href = url;
      link.download = `diagnostic-${artifact.artifact_id}.txt`;
      link.click();
      URL.revokeObjectURL(url);
    } catch {
      if (version === this.version) this.error.set('下载未完成或摘要不符，请刷新清单后重试。');
    } finally {
      if (version === this.version) this.busy.set(false);
    }
  }
  private async full(artifact: Artifact): Promise<Uint8Array<ArrayBuffer>> {
    const parts: string[] = [];
    let offset = 0;
    for (;;) {
      const page = await this.readChunk(artifact, offset);
      parts.push(page.text);
      offset = page.next;
      if (page.end) break;
    }
    const bytes = new TextEncoder().encode(parts.join(''));
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const hash = Array.from(digest, (value) => value.toString(16).padStart(2, '0')).join('');
    if (bytes.length !== artifact.export_bytes || hash !== artifact.export_sha256)
      throw new Error('下载摘要不符');
    return bytes;
  }
}

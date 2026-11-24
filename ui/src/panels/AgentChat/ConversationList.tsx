import { Bot, Loader2, Plus, AlertTriangle } from 'lucide-react'
import type { ChatConversationSummary } from '../../lib/chatConversationModel'
import { formatOriginLabel } from './conversations'
import './conversation-list.css'

export function ConversationList({
  conversations,
  note,
  activeConversationId,
  busy,
  onSelect,
  onNewChat,
}: {
  conversations: ChatConversationSummary[]
  note?: string | null
  activeConversationId: string | null
  busy: boolean
  onSelect: (id: string) => void
  onNewChat: () => void
}) {
  return (
    <section
      className="chat-convos"
      aria-label="Project conversations"
      data-cut-chat-conversation-list
    >
      <div className="chat-convos__head">
        <span className="chat-convos__title">
          Conversations
          {conversations.length > 0 && (
            <span className="chat-convos__count">{conversations.length}</span>
          )}
        </span>
        <button
          type="button"
          className="chat-convos__new"
          disabled={busy}
          onClick={onNewChat}
          data-cut-chat-new
          title="Start a new conversation"
        >
          <Plus size={12} strokeWidth={2} />
          New chat
        </button>
      </div>
      {conversations.length === 0 ? (
        <div className="chat-convos__empty">No saved conversations yet</div>
      ) : (
        <div className="chat-convos__items" role="list">
          {conversations.map((conv) => {
            const selected = conv.id === activeConversationId
            const originLabel = formatOriginLabel(conv.origin)
            return (
              <button
                key={conv.id}
                type="button"
                role="listitem"
                className={`chat-convos__item${selected ? ' is-selected' : ''}${!conv.readable ? ' is-unreadable' : ''}`}
                aria-current={selected ? 'true' : undefined}
                data-cut-chat-conversation={conv.id}
                data-cut-chat-conversation-origin={conv.origin}
                onClick={() => onSelect(conv.id)}
              >
                <div className="chat-convos__row-top">
                  <span className="chat-convos__item-title">{conv.title}</span>
                  {conv.running && (
                    <span
                      className="chat-convos__badge chat-convos__badge--running"
                      data-cut-chat-conversation-running
                    >
                      <Loader2 size={10} className="chat__spin" />
                      Running
                    </span>
                  )}
                </div>
                {(originLabel || !conv.readable) && (
                  <div className="chat-convos__row-meta">
                    {originLabel && (
                      <span className="chat-convos__badge chat-convos__badge--agent">
                        <Bot size={10} strokeWidth={1.75} />
                        {originLabel}
                      </span>
                    )}
                    {!conv.readable && (
                      <span
                        className="chat-convos__reason"
                        data-cut-chat-conversation-unreadable
                      >
                        <AlertTriangle size={10} strokeWidth={1.75} />
                        {conv.unreadable_reason || 'This conversation cannot be displayed.'}
                      </span>
                    )}
                  </div>
                )}
              </button>
            )
          })}
          {note && (
            <div className="chat-convos__note" data-cut-chat-conversation-more-note>
              {note}
            </div>
          )}
        </div>
      )}
    </section>
  )
}

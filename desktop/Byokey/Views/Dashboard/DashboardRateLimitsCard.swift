import SwiftUI

struct DashboardRateLimitsCard: View {
    let data: RateLimitsResponse

    var body: some View {
        Card("RATE LIMITS") {
            VStack(spacing: 8) {
                ForEach(data.providers, id: \.id) { provider in
                    ForEach(provider.accounts, id: \.account_id) { account in
                        if !account.snapshot.headers.isEmpty {
                            rateLimitRow(
                                name: provider.display_name,
                                multiAccount: provider.accounts.count > 1,
                                accountId: account.account_id,
                                headers: account.snapshot.headers,
                                capturedAt: account.snapshot.captured_at
                            )
                        }
                    }
                }
            }
        }
    }

    private func rateLimitRow(
        name: String, multiAccount: Bool, accountId: String,
        headers: [String: String], capturedAt: UInt64
    ) -> some View {
        let remaining = findHeader(headers, "remaining")
        let limit = findHeader(headers, "limit")

        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(name)
                    .fontWeight(.medium)
                if multiAccount {
                    Text("(\(accountId))")
                        .foregroundStyle(.tertiary)
                }
                Spacer()
                Text(timeAgo(capturedAt))
                    .foregroundStyle(.tertiary)
            }
            .font(.caption)

            if let remaining, let limit,
               let r = Double(remaining), let l = Double(limit), l > 0
            {
                HStack(spacing: 8) {
                    GeometryReader { geo in
                        ZStack(alignment: .leading) {
                            RoundedRectangle(cornerRadius: 3)
                                .fill(.quaternary)
                            RoundedRectangle(cornerRadius: 3)
                                .fill(ratioColor(r / l).gradient)
                                .frame(width: geo.size.width * r / l)
                        }
                    }
                    .frame(height: 6)

                    Text("\(remaining)/\(limit)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
            }
        }
    }

    private func findHeader(_ headers: [String: String], _ keyword: String) -> String? {
        headers.first(where: {
            $0.key.localizedCaseInsensitiveContains(keyword)
                && $0.key.localizedCaseInsensitiveContains("request")
        })?.value
    }

    private func ratioColor(_ ratio: Double) -> Color {
        if ratio > 0.5 { .green }
        else if ratio > 0.2 { .orange }
        else { .red }
    }

    private func timeAgo(_ ts: UInt64) -> String {
        let elapsed = Int64(Date().timeIntervalSince1970) - Int64(ts)
        if elapsed < 60 { return "just now" }
        if elapsed < 3600 { return "\(elapsed / 60)m ago" }
        return "\(elapsed / 3600)h ago"
    }
}

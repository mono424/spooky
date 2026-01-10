import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  CartesianGrid,
} from 'recharts';
import { data10, data100, data1000, data10000 } from './benchmarkData';

interface ChartProps {
  data: { record_count: number; latency_ms: number; is_extrapolated: number }[];
  title: string;
}

const Chart = ({ data, title }: ChartProps) => (
  <div className="bg-zinc-900 rounded-lg p-4 border border-zinc-800">
    <h3 className="text-sm font-medium text-zinc-400 mb-4">{title}</h3>
    <div className="h-[200px] w-full">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data}>
          <CartesianGrid strokeDasharray="3 3" stroke="#333" vertical={false} />
          <XAxis
            dataKey="record_count"
            stroke="#666"
            tick={{ fill: '#666', fontSize: 10 }}
            tickFormatter={(value) => `${value / 1000}k`}
            axisLine={false}
            tickLine={false}
          />
          <YAxis
            stroke="#666"
            tick={{ fill: '#666', fontSize: 10 }}
            axisLine={false}
            tickLine={false}
          />
          <Tooltip
            contentStyle={{
              backgroundColor: '#18181b',
              border: '1px solid #27272a',
              borderRadius: '6px',
            }}
            itemStyle={{ color: '#e4e4e7' }}
            labelStyle={{ color: '#a1a1aa' }}
            formatter={(value: number | undefined) => [
              `${(value ?? 0).toFixed(4)} ms`,
              'Latency',
            ]}
          />
          <Line
            type="monotone"
            dataKey="latency_ms"
            stroke="#a855f7"
            strokeWidth={2}
            dot={false}
            activeDot={{ r: 4, fill: '#a855f7' }}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  </div>
);

export default function BenchmarkCharts() {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-6 not-prose">
      <Chart data={data10} title="10 Registered Views" />
      <Chart data={data100} title="100 Registered Views" />
      <Chart data={data1000} title="1,000 Registered Views" />
      <Chart data={data10000} title="10,000 Registered Views" />
    </div>
  );
}

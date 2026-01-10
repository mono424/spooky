import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  CartesianGrid,
} from 'recharts';

const data10 = [
  { record_count: 0, latency_ms: 0.0338 },
  { record_count: 5000, latency_ms: 0.0288 },
  { record_count: 10000, latency_ms: 0.0251 },
  { record_count: 15000, latency_ms: 0.0275 },
  { record_count: 20000, latency_ms: 0.0269 },
  { record_count: 25000, latency_ms: 0.0298 },
  { record_count: 30000, latency_ms: 0.0291 },
  { record_count: 35000, latency_ms: 0.0292 },
  { record_count: 40000, latency_ms: 0.0257 },
  { record_count: 45000, latency_ms: 0.0241 },
  { record_count: 50000, latency_ms: 0.026 },
  { record_count: 55000, latency_ms: 0.024 },
  { record_count: 60000, latency_ms: 0.0254 },
  { record_count: 65000, latency_ms: 0.028 },
  { record_count: 70000, latency_ms: 0.0274 },
  { record_count: 75000, latency_ms: 0.027 },
  { record_count: 80000, latency_ms: 0.0244 },
  { record_count: 85000, latency_ms: 0.0224 },
  { record_count: 90000, latency_ms: 0.0269 },
  { record_count: 95000, latency_ms: 0.0255 },
  { record_count: 100000, latency_ms: 0.024 },
];

const data100 = [
  { record_count: 0, latency_ms: 0.0506 },
  { record_count: 5000, latency_ms: 0.0382 },
  { record_count: 10000, latency_ms: 0.0537 },
  { record_count: 15000, latency_ms: 0.0462 },
  { record_count: 20000, latency_ms: 0.0549 },
  { record_count: 25000, latency_ms: 0.0407 },
  { record_count: 30000, latency_ms: 0.046 },
  { record_count: 35000, latency_ms: 0.041 },
  { record_count: 40000, latency_ms: 0.0452 },
  { record_count: 45000, latency_ms: 0.0463 },
  { record_count: 50000, latency_ms: 0.0421 },
  { record_count: 55000, latency_ms: 0.0423 },
  { record_count: 60000, latency_ms: 0.0408 },
  { record_count: 65000, latency_ms: 0.0457 },
  { record_count: 70000, latency_ms: 0.0396 },
  { record_count: 75000, latency_ms: 0.04 },
  { record_count: 80000, latency_ms: 0.041 },
  { record_count: 85000, latency_ms: 0.0419 },
  { record_count: 90000, latency_ms: 0.0403 },
  { record_count: 95000, latency_ms: 0.0408 },
  { record_count: 100000, latency_ms: 0.0425 },
];

const data1000 = [
  { record_count: 0, latency_ms: 0.1157 },
  { record_count: 5000, latency_ms: 0.101 },
  { record_count: 10000, latency_ms: 0.1233 },
  { record_count: 15000, latency_ms: 0.1027 },
  { record_count: 20000, latency_ms: 0.099 },
  { record_count: 25000, latency_ms: 0.0893 },
  { record_count: 30000, latency_ms: 0.0903 },
  { record_count: 35000, latency_ms: 0.0866 },
  { record_count: 40000, latency_ms: 0.1092 },
  { record_count: 45000, latency_ms: 0.1113 },
  { record_count: 50000, latency_ms: 0.0955 },
  { record_count: 55000, latency_ms: 0.0925 },
  { record_count: 60000, latency_ms: 0.1105 },
  { record_count: 65000, latency_ms: 0.1021 },
  { record_count: 70000, latency_ms: 0.1208 },
  { record_count: 75000, latency_ms: 0.1007 },
  { record_count: 80000, latency_ms: 0.091 },
  { record_count: 85000, latency_ms: 0.1035 },
  { record_count: 90000, latency_ms: 0.1061 },
  { record_count: 95000, latency_ms: 0.1149 },
  { record_count: 100000, latency_ms: 0.106 },
];

const data10000 = [
  { record_count: 0, latency_ms: 0.4528 },
  { record_count: 5000, latency_ms: 0.449 },
  { record_count: 10000, latency_ms: 0.4648 },
  { record_count: 15000, latency_ms: 0.4489 },
  { record_count: 20000, latency_ms: 0.4459 },
  { record_count: 25000, latency_ms: 0.441 },
  { record_count: 30000, latency_ms: 0.4454 },
  { record_count: 35000, latency_ms: 0.4606 },
  { record_count: 40000, latency_ms: 0.4672 },
  { record_count: 45000, latency_ms: 0.4542 },
  { record_count: 50000, latency_ms: 0.449 },
  { record_count: 55000, latency_ms: 0.4406 },
  { record_count: 60000, latency_ms: 0.4898 },
  { record_count: 65000, latency_ms: 0.4432 },
  { record_count: 70000, latency_ms: 0.4956 },
  { record_count: 75000, latency_ms: 0.4396 },
  { record_count: 80000, latency_ms: 0.5609 },
  { record_count: 85000, latency_ms: 0.4582 },
  { record_count: 90000, latency_ms: 0.4431 },
  { record_count: 95000, latency_ms: 0.4464 },
  { record_count: 100000, latency_ms: 0.452 },
];

interface ChartProps {
  data: { record_count: number; latency_ms: number }[];
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
            domain={[0, 0.6]}
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
